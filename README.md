# foxcore-desktop-pet

FoxCore（FoxNature）桌面桌宠插件：一只由**活力值**驱动动画状态的狐狸，以透明无边框置顶窗常驻桌面，并通过 `desktop-pet` 适配器接入 FoxCore 消息管线。

- **GUI**：eframe / egui，狐狸用 egui 形状绘制（无图片 / SVG 光栅化依赖）
- **UI 配色**：kitsuneflora「暖狐毛」fox-orange 主题（`#c2632b` / `#e8945a`），不复用 mofox 蓝
- **ABI 版本**：1.6（对应 FoxNature v0.2.0 / Plugin SDK 0.2.0）

## 核心概念

### 活力值（vitality）

活力值是插件本地的复合值，锚定主程序只读的「精力」（energy）：

```text
vitality = clamp(energy + interaction + day_night, 0, 100)
  energy       来自主程序 /metrics 的 foxcore_sleep_energy_percent
  interaction  interaction_bonus × 0.5^(距上次交互分钟 / decay_half_life_minutes)
  day_night    白天 +day_bonus，夜间 -night_penalty
```

活力值驱动动画状态机：

| 阶段 | 活力值 | 表现 |
|------|-------|------|
| `energetic` | ≥ 80 | 跳跃 / 摇摆 |
| `active` | 60–80 | 眨眼 + 小动作 |
| `idle` | 30–60 | 静止站立 |
| `drowsy` | 15–30 | 打哈欠 / 垂头 |
| `sleeping` | < 15 | 闭眼 + Zzz |

### 数据流

```
                 host Tokio runtime
┌──────────────────────────────────────────────┐
│  DesktopPetPlugin                            │
│  ├─ vitality_loop  轮询 /metrics → 活力值     │
│  ├─ event_loop     try_recv GUI 事件          │
│  │    UserMessage ──▶ IncomingMessage ──▶ emit │
│  │    Petted      ──▶ 交互加成（衰减重置）     │
│  └─ adapter_send_message                     │
│       OutgoingMessage ──▶ 气泡文本 ──▶ GUI     │
└──────────────┬───────────────────────────────┘
               │ std::sync::mpsc
┌──────────────▼───────────────────────────────┐
│  GUI 线程（std::thread，eframe::run_native）    │
│  透明无边框置顶窗：狐狸 + 气泡 + 聊天输入框      │
└──────────────────────────────────────────────┘
```

## 构建与打包

依赖：Rust 1.92.0（见 `rust-toolchain.toml`），构建需要联网拉取 SDK 与 crates.io 依赖。

```bash
# 编译当前平台动态库
cargo build --release

# Linux 需先安装 eframe/egui 的系统依赖（见 CI 的 apt 列表）
```

打包为 `.foxplugin`（推荐直接交由 GitHub Actions 的 `release.yml` 矩阵编译 + 打包）。手动打包需先取 `foxplugin` CLI：

```bash
# CLI 从公开 release 下载（KitsuneFlora/foxcore-plugin-sdk 的 foxplugin-v0.3.0）
foxplugin pack . --output dist/foxcore-desktop-pet.foxplugin
```

产物为 `foxcore_desktop_pet.dll`（Windows）/ `libfoxcore_desktop_pet.so`（Linux），复制到 FoxNature 实例的 `plugins/` 即可加载。

## 配置

主程序会自动依据 `default-config.toml` 生成 `config/plugins/foxcore-desktop-pet.toml`：

```toml
version = 1

# ── 精力同步 ──
metrics_url = "http://127.0.0.1:8080/metrics"   # 读取 foxcore_sleep_energy_percent
poll_energy_secs = 30

# ── 活力值引擎 ──
interaction_bonus = 12.0
decay_half_life_minutes = 20.0
day_bonus = 5.0
night_penalty = 15.0

# ── 昼夜作息 ──
sleep_enabled = true
sleep_start_hour = 23
wake_start_hour = 7
utc_offset_hours = 8

# ── 聊天 ──
user_name = "用户"
pet_name = "桌宠"
system_prompt = "…"
dialog_auto_hide_sec = 10.0
show_chat_messages = false

# ── 主题 ──
theme_mode = "dark"     # dark / light
opacity = 1.0
```

## 交互

- **点击 / 抚摸狐狸**：触发一次交互加成（`interaction_bonus`），活力值瞬时回升并衰减
- **输入框回车**：把文字作为入站消息交给核心 agent，回复显示为头顶气泡
- **拖拽狐狸**：移动窗口
- **双击窗口区域外**：无操作（窗口为 frameless，退出由主程序停止插件时触发）

## v1 边界

- 仅文本消息；媒体段（图片 / 语音等）降级为占位文本
- 狐狸为 egui 形状简笔画，动画为程序化正弦/衰减驱动
- macOS 暂不支持（eframe 需主线程事件循环）；Android / aarch64 无桌面 GUI 不纳入 CI

## 许可

AGPL-3.0-or-later，见 [LICENSE](LICENSE)。
