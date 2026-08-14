//! GUI 线程与异步侧（host Tokio runtime）之间的消息类型。
//!
//! 两条单向通路：
//! - 异步侧 → GUI：`GuiCommand`（`std::sync::mpsc`；气泡、聊天、活力值快照、退出）
//! - GUI → 异步侧：`GuiEvent`（UDP 桥，JSON datagram；用户消息、抚摸交互）

use serde::{Deserialize, Serialize};

use crate::vitality::VitalityState;

/// 异步侧发给 GUI 线程的命令。
#[derive(Debug, Clone)]
pub enum GuiCommand {
    /// 桌宠头顶气泡显示文本。
    ShowDialog(String),
    /// 追加聊天记录（role：`user` / `pet`）。
    AppendChat { role: String, text: String },
    /// 更新活力值快照（驱动动画状态机）。
    SetVitality(VitalityState),
    /// 关闭 GUI 窗口并结束 GUI 线程。
    Quit,
}

/// GUI 线程发给异步侧的事件（经 UDP 桥序列化为 JSON datagram）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GuiEvent {
    /// 用户在聊天框输入并发送的消息。
    UserMessage(String),
    /// 用户点击/抚摸桌宠，触发一次交互加成。
    Petted,
}
