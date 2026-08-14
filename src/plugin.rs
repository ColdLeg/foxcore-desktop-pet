//! AbiPlugin 实现：DesktopPetPlugin 的生命周期、适配器方法与后台任务。
//!
//! 后台任务分三类：
//! - GUI 线程（`std::thread`）：`eframe::run_native`，与 Tokio runtime 解耦；
//! - 活力值循环（host task）：轮询主程序 `/metrics` 精力 → 计算活力值 → 持久化；
//! - 事件循环（host task）：`try_recv` 拉取 GUI 事件 → 入站消息 / 交互加成。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use foxcore_plugin_sdk::abi_stable::std_types::{ROption, RResult, RString, RVec};
use foxcore_plugin_sdk::async_ffi::FfiFuture;
use foxcore_plugin_sdk::{
    AbiConversationYield, AbiError, AbiErrorCode, AbiLogEvent, AbiLogLevel, AbiPlugin,
    AdapterCallbackBox, AdapterDescriptor, AdapterEvent, ConversationContextBox, HostApi,
    PluginCapabilities,
    catch_panic, guarded_async, guarded_fire_and_forget,
};

use crate::channels::{GuiCommand, GuiEvent};
use crate::config::{CONFIG_VERSION, DesktopPetConfig};
use crate::convert::{self, ADAPTER_NAME};
use crate::vitality::{self, VitalityState};

const DEFAULT_CONFIG_TOML: &str = include_str!("../default-config.toml");
const STATE_KEY: &str = "vitality";

pub struct DesktopPetPlugin {
    host: Arc<HostApi>,
    config: Mutex<DesktopPetConfig>,
    callback: Mutex<Option<Arc<AdapterCallbackBox>>>,
    vitality: Arc<Mutex<VitalityState>>,
    stop_flag: Arc<AtomicBool>,
    // 异步侧 → GUI 的命令通道（Sender 可克隆共享）。
    tx_gui: Sender<GuiCommand>,
    // GUI 线程独占的接收端（adapter_start 时取出移入线程）。
    rx_cmd: Mutex<Option<Receiver<GuiCommand>>>,
    // GUI → 异步侧的事件通道（Sender 移入 GUI 线程，Receiver 移入事件循环）。
    tx_event: Mutex<Option<Sender<GuiEvent>>>,
    rx_gui: Mutex<Option<Receiver<GuiEvent>>>,
    vitality_task: Mutex<Option<u64>>,
    event_task: Mutex<Option<u64>>,
    gui_handle: Mutex<Option<JoinHandle<()>>>,
}

impl DesktopPetPlugin {
    pub fn new(host: Arc<HostApi>, config: DesktopPetConfig) -> Self {
        let (tx_gui, rx_cmd) = channel::<GuiCommand>();
        let (tx_event, rx_gui) = channel::<GuiEvent>();
        Self {
            host,
            config: Mutex::new(config),
            callback: Mutex::new(None),
            vitality: Arc::new(Mutex::new(VitalityState::default())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            tx_gui,
            rx_cmd: Mutex::new(Some(rx_cmd)),
            tx_event: Mutex::new(Some(tx_event)),
            rx_gui: Mutex::new(Some(rx_gui)),
            vitality_task: Mutex::new(None),
            event_task: Mutex::new(None),
            gui_handle: Mutex::new(None),
        }
    }

    /// 同步地完成适配器启动：存回调、起 GUI 线程、挂两个后台任务。
    fn start_adapter(&self, callback: AdapterCallbackBox) -> Result<(), AbiError> {
        let host = Arc::clone(&self.host);
        let config = self.config.lock().unwrap().clone();
        let callback_arc = Arc::new(callback);
        let vitality = Arc::clone(&self.vitality);
        let stop_flag = Arc::clone(&self.stop_flag);

        let rx_cmd = self
            .rx_cmd
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| AbiError::internal("GUI 命令通道已被消费"))?;
        let tx_event = self
            .tx_event
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| AbiError::internal("GUI 事件通道已被消费"))?;
        let rx_gui = self
            .rx_gui
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| AbiError::internal("GUI 事件通道已被消费"))?;

        *self.callback.lock().unwrap() = Some(Arc::clone(&callback_arc));

        // GUI 线程
        let gui_config = config.clone();
        let handle = std::thread::spawn(move || {
            crate::gui::run_gui(rx_cmd, tx_event, gui_config);
        });
        *self.gui_handle.lock().unwrap() = Some(handle);

        // 活力值循环
        let vitality_task = host
            .task
            .spawn(
                RString::from("desktop-pet-vitality"),
                guarded_fire_and_forget(vitality_loop(
                    Arc::clone(&host),
                    config.clone(),
                    Arc::clone(&vitality),
                    self.tx_gui.clone(),
                    Arc::clone(&stop_flag),
                )),
            )
            .into_result()?;
        *self.vitality_task.lock().unwrap() = Some(vitality_task);

        // 事件循环
        let event_task = host
            .task
            .spawn(
                RString::from("desktop-pet-events"),
                guarded_fire_and_forget(event_loop(
                    Arc::clone(&host),
                    config.clone(),
                    Arc::clone(&callback_arc),
                    Arc::clone(&vitality),
                    self.tx_gui.clone(),
                    rx_gui,
                    Arc::clone(&stop_flag),
                )),
            )
            .into_result()?;
        *self.event_task.lock().unwrap() = Some(event_task);

        host.log.log(AbiLogEvent::message(
            AbiLogLevel::Info,
            "桌宠",
            format!(
                "adapter `{ADAPTER_NAME}` started（vitality={vitality_task}, events={event_task}）"
            ),
        ));

        Ok(())
    }

    /// 停止后台任务与 GUI；`join_gui` 为真时等待 GUI 线程退出。
    fn stop_inner(&self, join_gui: bool) -> Result<(), AbiError> {
        self.stop_flag.store(true, Ordering::Release);
        if let Some(id) = self.vitality_task.lock().unwrap().take() {
            self.host.task.abort(id);
        }
        if let Some(id) = self.event_task.lock().unwrap().take() {
            self.host.task.abort(id);
        }
        let _ = self.tx_gui.send(GuiCommand::Quit);
        if join_gui {
            if let Some(handle) = self.gui_handle.lock().unwrap().take() {
                let _ = handle.join();
            }
        }
        Ok(())
    }
}

/// 活力值循环：轮询精力 → 计算活力值 → 持久化 → 推送 GUI。
async fn vitality_loop(
    host: Arc<HostApi>,
    config: DesktopPetConfig,
    vitality: Arc<Mutex<VitalityState>>,
    tx_gui: Sender<GuiCommand>,
    stop_flag: Arc<AtomicBool>,
) {
    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
        let energy = vitality::poll_energy(&host.http, &config.metrics_url).await;
        let now = vitality::unix_seconds();
        let state = {
            let current = *vitality.lock().unwrap();
            vitality::compute_vitality(&current, &config, now, energy)
        };
        *vitality.lock().unwrap() = state;
        let _ = tx_gui.send(GuiCommand::SetVitality(state));
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = host
                .state
                .save(RString::from(STATE_KEY), RString::from(json))
                .await;
        }
        host.time.sleep_ms(config.poll_energy_secs * 1000).await;
    }
}

/// 事件循环：拉取 GUI 事件 → 入站消息上报 / 抚摸加成。
async fn event_loop(
    host: Arc<HostApi>,
    config: DesktopPetConfig,
    callback: Arc<AdapterCallbackBox>,
    vitality: Arc<Mutex<VitalityState>>,
    tx_gui: Sender<GuiCommand>,
    rx_gui: Receiver<GuiEvent>,
    stop_flag: Arc<AtomicBool>,
) {
    loop {
        if stop_flag.load(Ordering::Relaxed) {
            break;
        }
        match rx_gui.try_recv() {
            Ok(GuiEvent::UserMessage(text)) => {
                let event = convert::incoming_from_text(text, &config);
                if let Ok(json) = foxcore_plugin_sdk::encode_json("AdapterEvent", &event) {
                    if let AdapterEvent::MessageReceived(msg) = &event {
                        if let Ok(incoming) =
                            foxcore_plugin_sdk::encode_json("IncomingMessage", msg.as_ref())
                        {
                            callback.observe_incoming(incoming);
                        }
                    }
                    callback.emit(json).await;
                }
            }
            Ok(GuiEvent::Petted) => {
                let now = vitality::unix_seconds();
                let state = {
                    let mut current = *vitality.lock().unwrap();
                    current.last_interaction_secs = now;
                    vitality::compute_vitality(&current, &config, now, None)
                };
                *vitality.lock().unwrap() = state;
                let _ = tx_gui.send(GuiCommand::SetVitality(state));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
        host.time.sleep_ms(100).await;
    }
}

impl AbiPlugin for DesktopPetPlugin {
    fn capabilities(&self) -> RResult<PluginCapabilities, AbiError> {
        catch_panic(|| {
            Ok(PluginCapabilities {
                tools: RVec::new(),
                adapters: RVec::from(vec![AdapterDescriptor {
                    name: RString::from(ADAPTER_NAME),
                    description: RString::from("FoxCore 桌面桌宠（透明置顶窗 + 活力值引擎）"),
                    inbound_segments: RVec::from(vec![RString::from("text")]),
                    outbound_segments: RVec::from(vec![
                        RString::from("text"),
                        RString::from("markdown"),
                    ]),
                }]),
                conversations: RVec::new(),
                control: false,
            })
        })
        .into()
    }

    fn initialize(&self) -> FfiFuture<RResult<(), AbiError>> {
        let host = Arc::clone(&self.host);
        let config = self.config.lock().unwrap().clone();
        let vitality = Arc::clone(&self.vitality);
        let tx_gui = self.tx_gui.clone();
        guarded_async(async move {
            if config.version < CONFIG_VERSION {
                host.log.log(AbiLogEvent::message(
                    AbiLogLevel::Info,
                    "桌宠",
                    format!("config version {} < {CONFIG_VERSION}，写入默认配置", config.version),
                ));
                let _ = host.config.save(RString::from(DEFAULT_CONFIG_TOML));
            }

            if let Ok(ROption::RSome(json)) =
                host.state.load(RString::from(STATE_KEY)).await.into_result()
            {
                if let Ok(state) = serde_json::from_str::<VitalityState>(json.as_str()) {
                    *vitality.lock().unwrap() = state;
                    let _ = tx_gui.send(GuiCommand::SetVitality(state));
                }
            }

            host.log.log(AbiLogEvent::message(AbiLogLevel::Info, "桌宠", "plugin initialized"));
            Ok(())
        })
    }

    fn invoke_tool(
        &self,
        _tool_name: RString,
        _args_json: RString,
    ) -> FfiFuture<RResult<RString, AbiError>> {
        unsupported("tool")
    }

    fn adapter_start(
        &self,
        _adapter: RString,
        callback: AdapterCallbackBox,
    ) -> FfiFuture<RResult<(), AbiError>> {
        let result = catch_panic(|| self.start_adapter(callback));
        guarded_async(async move { result })
    }

    fn adapter_send_message(
        &self,
        _adapter: RString,
        outgoing_json: RString,
    ) -> FfiFuture<RResult<RString, AbiError>> {
        let tx_gui = self.tx_gui.clone();
        let json = outgoing_json.to_string();
        guarded_async(async move {
            let text = convert::outgoing_to_text(&json)?;
            if !text.is_empty() {
                let _ = tx_gui.send(GuiCommand::ShowDialog(text.clone()));
                let _ = tx_gui.send(GuiCommand::AppendChat {
                    role: "pet".to_string(),
                    text,
                });
            }
            Ok(RString::from(convert::next_message_id()))
        })
    }

    fn adapter_call_api(
        &self,
        _adapter: RString,
        _action: RString,
        _params_json: RString,
    ) -> FfiFuture<RResult<RString, AbiError>> {
        unsupported("adapter call_api")
    }

    fn adapter_stop(&self, _adapter: RString) -> FfiFuture<RResult<(), AbiError>> {
        // 与 shutdown 一致：join GUI 线程，确保其 EventLoop 在返回前已 Drop，
        // 否则下一次热重载会撞上 winit 的 “EventLoop can't be recreated”。
        let result = catch_panic(|| self.stop_inner(true));
        guarded_async(async move { result })
    }

    fn conversation_factory_start(
        &self,
        _factory: RString,
    ) -> FfiFuture<RResult<(), AbiError>> {
        unsupported("conversation")
    }

    fn conversation_applies_to(
        &self,
        _factory: RString,
        _stream_json: RString,
    ) -> RResult<bool, AbiError> {
        unsupported_sync("conversation")
    }

    fn conversation_create(
        &self,
        _factory: RString,
        _stream_json: RString,
    ) -> RResult<u64, AbiError> {
        unsupported_sync("conversation")
    }

    fn conversation_execute(
        &self,
        _conversation_id: u64,
        _context: ConversationContextBox,
    ) -> FfiFuture<RResult<AbiConversationYield, AbiError>> {
        unsupported("conversation")
    }

    fn conversation_factory_control(
        &self,
        _factory: RString,
        _command: RString,
        _params_json: RString,
    ) -> FfiFuture<RResult<RString, AbiError>> {
        unsupported("conversation")
    }

    fn conversation_drop(&self, _conversation_id: u64) -> RResult<(), AbiError> {
        unsupported_sync("conversation")
    }

    fn conversation_factory_stop(
        &self,
        _factory: RString,
    ) -> FfiFuture<RResult<(), AbiError>> {
        unsupported("conversation")
    }

    fn handle_control(
        &self,
        _command: RString,
        _params_json: RString,
    ) -> FfiFuture<RResult<RString, AbiError>> {
        unsupported("control")
    }

    fn shutdown(&self) -> FfiFuture<RResult<(), AbiError>> {
        let result = catch_panic(|| self.stop_inner(true));
        guarded_async(async move { result })
    }
}

// ── 未声明能力的占位实现 ──────────────────────────────────────────────

fn unsupported<T: Send + 'static>(capability: &'static str) -> FfiFuture<RResult<T, AbiError>> {
    guarded_async(async move {
        Err(AbiError::new(
            AbiErrorCode::NotEnabled,
            format!("plugin does not declare {capability} capability"),
        ))
    })
}

fn unsupported_sync<T>(capability: &'static str) -> RResult<T, AbiError> {
    RResult::RErr(AbiError::new(
        AbiErrorCode::NotEnabled,
        format!("plugin does not declare {capability} capability"),
    ))
}
