//! 活力值引擎（纯逻辑）。
//!
//! 活力值 = 主程序精力（energy）为底，叠加交互加成（指数衰减）与昼夜修正，
//! 钳制在 `[0, 100]`。活力值驱动桌宠动画状态机。精力只读，来自主程序
//! `/metrics` 的 `foxcore_sleep_energy_percent` gauge。

use std::time::{SystemTime, UNIX_EPOCH};

use foxcore_plugin_sdk::abi_stable::std_types::{ROption, RString, RVec};
use foxcore_plugin_sdk::{HostHttpRef, HttpRequest};
use serde::{Deserialize, Serialize};

use crate::config::DesktopPetConfig;

/// 桌宠动画状态机（由活力值驱动）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VitalityStage {
    /// 精力充沛：跳跃 / 摇摆。
    Energetic,
    /// 活跃：正常眨眼 + 随机小动作。
    Active,
    /// 闲适：静止站立。
    Idle,
    /// 困倦：打哈欠 / 垂头。
    Drowsy,
    /// 睡眠：闭眼 + Zzz。
    Sleeping,
}

impl VitalityStage {
    pub fn from_vitality(v: f32) -> Self {
        if v >= 80.0 {
            Self::Energetic
        } else if v >= 60.0 {
            Self::Active
        } else if v >= 30.0 {
            Self::Idle
        } else if v >= 15.0 {
            Self::Drowsy
        } else {
            Self::Sleeping
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Energetic => "energetic",
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Drowsy => "drowsy",
            Self::Sleeping => "sleeping",
        }
    }
}

/// 活力值快照（引擎与 GUI 之间共享，也用于 `host.state` 持久化）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct VitalityState {
    pub vitality: f32,
    pub energy: f32,
    pub stage: VitalityStage,
    pub is_day: bool,
    pub last_interaction_secs: i64,
}

impl Default for VitalityState {
    fn default() -> Self {
        Self {
            vitality: 100.0,
            energy: 100.0,
            stage: VitalityStage::Energetic,
            is_day: true,
            last_interaction_secs: 0,
        }
    }
}

/// 当前 Unix 秒。
pub fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 由 Unix 秒与 UTC 偏移计算本地小时（0-23）。
pub fn local_hour(unix_secs: i64, utc_offset_hours: i32) -> u32 {
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let utc_hour = (secs_of_day / 3_600) as i32;
    ((utc_hour + utc_offset_hours) % 24 + 24) % 24
}

/// 是否处于白天（醒着时段），支持跨午夜作息。
pub fn is_daytime(local_hour: u32, sleep_start: u32, wake_start: u32) -> bool {
    if wake_start < sleep_start {
        local_hour >= wake_start && local_hour < sleep_start
    } else {
        // 跨午夜：例如 sleep=2, wake=22
        local_hour >= wake_start || local_hour < sleep_start
    }
}

/// 由当前状态、精力与配置计算新活力值。
pub fn compute_vitality(
    state: &VitalityState,
    config: &DesktopPetConfig,
    now_secs: i64,
    energy: Option<f32>,
) -> VitalityState {
    let energy = energy.unwrap_or(state.energy).clamp(0.0, 100.0);

    let elapsed_min = ((now_secs - state.last_interaction_secs).max(0) as f32) / 60.0;
    let half_life = config.decay_half_life_minutes.max(0.1);
    let decay_factor = 0.5_f32.powf(elapsed_min / half_life);

    let interaction = config.interaction_bonus * decay_factor;

    let hour = local_hour(now_secs, config.utc_offset_hours);
    let is_day = if config.sleep_enabled {
        is_daytime(hour, config.sleep_start_hour, config.wake_start_hour)
    } else {
        true
    };
    let day_night = if is_day {
        config.day_bonus
    } else {
        -config.night_penalty
    };

    let vitality = (energy + interaction + day_night).clamp(0.0, 100.0);

    VitalityState {
        vitality,
        energy,
        stage: VitalityStage::from_vitality(vitality),
        is_day,
        last_interaction_secs: state.last_interaction_secs,
    }
}

/// 从主程序 `/metrics` 端点读取精力值；失败返回 `None`。
pub async fn poll_energy(http: &HostHttpRef, metrics_url: &str) -> Option<f32> {
    let req = HttpRequest {
        method: RString::from("GET"),
        url: RString::from(metrics_url),
        headers: RVec::new(),
        body: RVec::new(),
        timeout_ms: ROption::RSome(5_000),
        max_response_bytes: ROption::RSome(1024 * 1024),
    };
    let resp = http.request(req).await.into_result().ok()?;
    if resp.status != 200 {
        return None;
    }
    let body = String::from_utf8_lossy(resp.body.as_slice());
    parse_energy_metric(&body)
}

/// 在 Prometheus 文本中查找 `foxcore_sleep_energy_percent` gauge 值。
fn parse_energy_metric(text: &str) -> Option<f32> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("foxcore_sleep_energy_percent") {
            if let Some(value) = rest.split_whitespace().next() {
                return value.parse::<f32>().ok();
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_prometheus_gauge() {
        let text = "# HELP foo\nfoxcore_sleep_energy_percent 99.35\n# TYPE\n";
        assert_eq!(parse_energy_metric(text), Some(99.35));
    }

    #[test]
    fn it_maps_stages_by_vitality() {
        assert_eq!(VitalityStage::from_vitality(90.0), VitalityStage::Energetic);
        assert_eq!(VitalityStage::from_vitality(70.0), VitalityStage::Active);
        assert_eq!(VitalityStage::from_vitality(45.0), VitalityStage::Idle);
        assert_eq!(VitalityStage::from_vitality(20.0), VitalityStage::Drowsy);
        assert_eq!(VitalityStage::from_vitality(5.0), VitalityStage::Sleeping);
    }

    #[test]
    fn it_detects_daytime_without_midnight_cross() {
        assert!(is_daytime(12, 23, 7));
        assert!(!is_daytime(0, 23, 7));
    }

    #[test]
    fn it_detects_daytime_with_midnight_cross() {
        assert!(is_daytime(23, 2, 22));
        assert!(!is_daytime(1, 2, 22));
    }
}
