//! AbiPlugin 实现：DesktopPetPlugin 的生命周期、适配器方法与后台任务。
//!
//! 后台任务分三类：
//! - GUI 线程（`std::thread`）：`eframe::run_native`，与 Tokio runtime 解耦；
//! - 活力值循环（host task）：轮询主程序 `/metrics` 精力 → 计算活力值 → 持久化；
//! - UDP 桥 receiver（`std::thread`）：阻塞 `recv_from`，收到 GUI datagram 即转发主程序。

use std::net::{SocketAddr, UdpSocket};
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
/// UDP 桥的退出哨兵 datagram；receiver 线程收到后结束 recv 循环。
const UDP_QUIT: &[u8] = b"__foxcore_desktop_pet_quit__";

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
    // GUI → 异步侧的 UDP 桥：GUI 线程把 GuiEvent 序列化为 JSON datagram 发到这里，
    // receiver 线程阻塞 recv_from，收到即转发主程序（无轮询）。
    udp_socket: Arc<UdpSocket>,
    udp_addr: SocketAddr,
    event_thread: Mutex<Option<JoinHandle<()>>>,
    vitality_task: Mutex<Option<u64>>,
    gui_handle: Mutex<Option<JoinHandle<()>>>,
}

impl DesktopPetPlugin {
    pub fn new(host: Arc<HostApi>, config: DesktopPetConfig) -> Self {
        let (tx_gui, rx_cmd) = channel::<GuiCommand>();
        let udp_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").expect("绑定桌宠 UDP 桥失败"));
        let udp_addr = udp_socket.local_addr().expect("读取 UDP 桥地址失败");
        Self {
            host,
            config: Mutex::new(config),
            callback: Mutex::new(None),
            vitality: Arc::new(Mutex::new(VitalityState::default())),
            stop_flag: Arc::new(AtomicBool::new(false)),
            tx_gui,
            rx_cmd: Mutex::new(Some(rx_cmd)),
            udp_socket,
            udp_addr,
            event_thread: Mutex::new(None),
            vitality_task: Mutex::new(None),
            gui_handle: Mutex::new(None),
        }
    }

    /// 同步地完成适配器启动：存回调、起 GUI 线程、UDP 桥 receiver 线程、活力值任务。
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

        *self.callback.lock().unwrap() = Some(Arc::clone(&callback_arc));

        // GUI 线程：通过 UDP 桥发送 GuiEvent。
        let gui_config = config.clone();
        let udp_addr = self.udp_addr;
        let gui_handle = std::thread::spawn(move || {
            crate::gui::run_gui(rx_cmd, udp_addr, gui_config);
        });
        *self.gui_handle.lock().unwrap() = Some(gui_handle);

        // UDP 桥 receiver 线程：阻塞 recv_from，收到 datagram 立即转发主程序。
        let recv_socket = Arc::clone(&self.udp_socket);
        let recv_host = Arc::clone(&host);
        let recv_callback = Arc::clone(&callback_arc);
        let recv_config = config.clone();
        let recv_vitality = Arc::clone(&vitality);
        let recv_tx_gui = self.tx_gui.clone();
        let event_thread = std::thread::spawn(move || {
            event_receiver(
                recv_socket,
                recv_host,
                recv_callback,
                recv_config,
                recv_vitality,
                recv_tx_gui,
            );
        });
        *self.event_thread.lock().unwrap() = Some(event_thread);

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

        host.log.log(AbiLogEvent::message(
            AbiLogLevel::Info,
            "桌宠",
            format!(
                "adapter `{ADAPTER_NAME}` started（vitality={vitality_task}, udp={}）",
                self.udp_addr
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
        // 唤醒并结束 UDP 桥 receiver 线程（哨兵 datagram 使 recv_from 返回）。
        let _ = self.udp_socket.send_to(UDP_QUIT, self.udp_addr);
        if let Some(handle) = self.event_thread.lock().unwrap().take() {
            let _ = handle.join();
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

/// UDP 桥 receiver：阻塞 `recv_from`，收到 datagram 即转发（无轮询）。
///
/// `emit` 是异步回调，必须由 host runtime poll，因此这里用 `host.task.spawn`
/// 把每条入站消息的广播交给 runtime；`observe_incoming` 是同步的，直接调用即可。
fn event_receiver(
    socket: Arc<UdpSocket>,
    host: Arc<HostApi>,
    callback: Arc<AdapterCallbackBox>,
    config: DesktopPetConfig,
    vitality: Arc<Mutex<VitalityState>>,
    tx_gui: Sender<GuiCommand>,
) {
    let mut buf = [0u8; 65536];
    loop {
        match socket.recv_from(&mut buf) {
            Ok((len, _src)) => {
                if &buf[..len] == UDP_QUIT {
                    break;
                }
                let Ok(event) = serde_json::from_slice::<GuiEvent>(&buf[..len]) else {
                    continue;
                };
                handle_gui_event(event, &host, &callback, &config, &vitality, &tx_gui);
            }
            Err(_) => break,
        }
    }
}

/// 处理单条 GUI 事件：入站消息上报主程序，或抚摸加成更新活力值。
fn handle_gui_event(
    event: GuiEvent,
    host: &Arc<HostApi>,
    callback: &Arc<AdapterCallbackBox>,
    config: &DesktopPetConfig,
    vitality: &Arc<Mutex<VitalityState>>,
    tx_gui: &Sender<GuiCommand>,
) {
    match event {
        GuiEvent::UserMessage(text) => {
            let event = convert::incoming_from_text(text, config);
            if let Ok(json) = foxcore_plugin_sdk::encode_json("AdapterEvent", &event) {
                if let AdapterEvent::MessageReceived(msg) = &event {
                    if let Ok(incoming) =
                        foxcore_plugin_sdk::encode_json("IncomingMessage", msg.as_ref())
                    {
                        callback.observe_incoming(incoming);
                    }
                }
                let host = Arc::clone(host);
                let callback = Arc::clone(callback);
                let _ = host.task.spawn(
                    RString::from("desktop-pet-emit"),
                    guarded_fire_and_forget(async move {
                        callback.emit(json).await;
                    }),
                );
            }
        }
        GuiEvent::Petted => {
            let now = vitality::unix_seconds();
            let state = {
                let mut current = *vitality.lock().unwrap();
                current.last_interaction_secs = now;
                vitality::compute_vitality(&current, config, now, None)
            };
            *vitality.lock().unwrap() = state;
            let _ = tx_gui.send(GuiCommand::SetVitality(state));
        }
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
