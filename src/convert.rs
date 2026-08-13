//! 桌宠 ↔ FoxCore 消息互转。
//!
//! 桌宠是本地 1:1 私聊通道：用户输入 → `IncomingMessage` → 核心 agent；
//! 核心生成的 `OutgoingMessage` → 桌宠气泡文本。

use std::sync::atomic::{AtomicU64, Ordering};

use foxcore_plugin_sdk::{
    AbiError, AdapterEvent, IncomingMessage, MessageAddressing, MessageSegment, MessageStream,
    OutgoingMessage, Sender, decode_json,
};

use crate::config::DesktopPetConfig;
use crate::vitality::unix_seconds;

/// 桌宠适配器名（对应 `MessageStream.adapter`）。
pub const ADAPTER_NAME: &str = "desktop-pet";
/// 桌宠会话流键（固定 1:1 私聊）。
pub const STREAM_KEY: &str = "desktop";

static MESSAGE_SEQ: AtomicU64 = AtomicU64::new(0);

/// 桌宠会话流定位。
#[must_use]
pub fn pet_stream() -> MessageStream {
    MessageStream::new(ADAPTER_NAME, "private", STREAM_KEY)
}

/// 生成递增且唯一的平台消息 id。
#[must_use]
pub fn next_message_id() -> String {
    let seq = MESSAGE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{ADAPTER_NAME}.{}.{seq}", unix_seconds())
}

/// 把用户在聊天框输入的文字转为入站事件。
#[must_use]
pub fn incoming_from_text(text: String, config: &DesktopPetConfig) -> AdapterEvent {
    let message = IncomingMessage::new(
        next_message_id(),
        pet_stream(),
        Sender::new("desktop-user", config.user_name.clone()),
        vec![MessageSegment::text(text.clone())],
        text,
        unix_seconds(),
    )
    .with_addressing(MessageAddressing::Direct);
    AdapterEvent::MessageReceived(Box::new(message))
}

/// 把出站消息 JSON 解码为桌宠气泡文本（多段降级拼接）。
///
/// # Errors
///
/// JSON 解码失败时返回 [`AbiError`]。
pub fn outgoing_to_text(outgoing_json: &str) -> Result<String, AbiError> {
    let outgoing: OutgoingMessage = decode_json("OutgoingMessage", outgoing_json)?;
    let mut parts: Vec<String> = Vec::with_capacity(outgoing.segments.len());
    for segment in &outgoing.segments {
        if let Some(text) = segment.try_textify() {
            parts.push(text);
        }
    }
    Ok(parts.join(""))
}
