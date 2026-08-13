//! 桌宠插件配置。对应 `config/plugins/foxcore-desktop-pet.toml`。

use serde::{Deserialize, Serialize};

/// 插件主配置。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DesktopPetConfig {
    /// 配置结构版本。
    pub version: u32,

    /// 主程序 /metrics 端点（读取精力）。
    #[serde(default = "default_metrics_url")]
    pub metrics_url: String,

    /// 精力轮询间隔（秒）。
    #[serde(default = "default_poll_energy_secs")]
    pub poll_energy_secs: u64,

    /// 每次交互的活力加成。
    #[serde(default = "default_interaction_bonus")]
    pub interaction_bonus: f32,

    /// 交互加成衰减半衰期（分钟）。
    #[serde(default = "default_decay_half_life_minutes")]
    pub decay_half_life_minutes: f32,

    /// 白天活力修正。
    #[serde(default = "default_day_bonus")]
    pub day_bonus: f32,

    /// 夜间活力修正（负值）。
    #[serde(default = "default_night_penalty")]
    pub night_penalty: f32,

    /// 是否启用昼夜作息。
    #[serde(default = "default_sleep_enabled")]
    pub sleep_enabled: bool,

    /// 入睡时间（小时 0-23）。
    #[serde(default = "default_sleep_start_hour")]
    pub sleep_start_hour: u32,

    /// 醒来时间（小时 0-23）。
    #[serde(default = "default_wake_start_hour")]
    pub wake_start_hour: u32,

    /// 本地时区相对 UTC 的偏移（小时）。
    #[serde(default = "default_utc_offset_hours")]
    pub utc_offset_hours: i32,

    /// 用户显示名称。
    #[serde(default = "default_user_name")]
    pub user_name: String,

    /// 桌宠显示名称。
    #[serde(default = "default_pet_name")]
    pub pet_name: String,

    /// 桌宠身份提示词。
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,

    /// 桌宠气泡自动隐藏时间（秒，0 不隐藏）。
    #[serde(default = "default_dialog_auto_hide_sec")]
    pub dialog_auto_hide_sec: f32,

    /// 是否显示聊天记录（v1 预留）。
    #[serde(default = "default_show_chat_messages")]
    pub show_chat_messages: bool,

    /// 主题模式：dark / light。
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,

    /// 窗口透明度（0.1~1.0）。
    #[serde(default = "default_opacity")]
    pub opacity: f32,
}

fn default_metrics_url() -> String {
    "http://127.0.0.1:8080/metrics".to_string()
}
fn default_poll_energy_secs() -> u64 {
    30
}
fn default_interaction_bonus() -> f32 {
    12.0
}
fn default_decay_half_life_minutes() -> f32 {
    20.0
}
fn default_day_bonus() -> f32 {
    5.0
}
fn default_night_penalty() -> f32 {
    15.0
}
fn default_sleep_enabled() -> bool {
    true
}
fn default_sleep_start_hour() -> u32 {
    23
}
fn default_wake_start_hour() -> u32 {
    7
}
fn default_utc_offset_hours() -> i32 {
    8
}
fn default_user_name() -> String {
    "用户".to_string()
}
fn default_pet_name() -> String {
    "桌宠".to_string()
}
fn default_system_prompt() -> String {
    "你当前正在以桌面宠物的形式运行在用户的电脑桌面上。\
你可以感知自己的活力值（精力+交互情绪）与昼夜状态。\
请以桌宠的身份与用户互动，保持自然、亲近的交流风格。"
        .to_string()
}
fn default_dialog_auto_hide_sec() -> f32 {
    10.0
}
fn default_show_chat_messages() -> bool {
    false
}
fn default_theme_mode() -> String {
    "dark".to_string()
}
fn default_opacity() -> f32 {
    1.0
}

impl Default for DesktopPetConfig {
    fn default() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            metrics_url: default_metrics_url(),
            poll_energy_secs: default_poll_energy_secs(),
            interaction_bonus: default_interaction_bonus(),
            decay_half_life_minutes: default_decay_half_life_minutes(),
            day_bonus: default_day_bonus(),
            night_penalty: default_night_penalty(),
            sleep_enabled: default_sleep_enabled(),
            sleep_start_hour: default_sleep_start_hour(),
            wake_start_hour: default_wake_start_hour(),
            utc_offset_hours: default_utc_offset_hours(),
            user_name: default_user_name(),
            pet_name: default_pet_name(),
            system_prompt: default_system_prompt(),
            dialog_auto_hide_sec: default_dialog_auto_hide_sec(),
            show_chat_messages: default_show_chat_messages(),
            theme_mode: default_theme_mode(),
            opacity: default_opacity(),
        }
    }
}

impl DesktopPetConfig {
    pub const CURRENT_VERSION: u32 = 1;
    pub const FILE_NAME: &str = "foxcore-desktop-pet.toml";
}

pub const CONFIG_VERSION: u32 = DesktopPetConfig::CURRENT_VERSION;
