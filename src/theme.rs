//! kitsuneflora「暖狐毛」配色。
//!
//! 颜色值取自 `kitsunefloraui` 的 `tokens.css`（fox-orange 品牌 + 暖奶油/深棕
//! 中性色），仅保留桌宠绘制与气泡所需的最小集合，不复用 mofox 蓝。

use egui::Color32;

/// 桌宠主题（浅色 / 深色两套）。
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// 窗口底色（透明窗口下主要用于气泡/面板背景）。
    pub bg: Color32,
    /// 卡片/面板表面色。
    pub surface: Color32,
    /// 边框色。
    pub border: Color32,
    /// 主文字色。
    pub text: Color32,
    /// 次文字色。
    pub text_muted: Color32,
    /// 品牌主色（狐狸毛发）。
    pub fur: Color32,
    /// 深一档毛发色（耳尖/阴影）。
    pub fur_dark: Color32,
    /// 奶油色（口鼻/肚皮）。
    pub cream: Color32,
    /// 眼睛/鼻头（墨色）。
    pub ink: Color32,
    /// 腮红/点缀。
    pub accent: Color32,
}

impl Theme {
    /// 按 `theme_mode` 选择主题；非 `light` 一律回退深色。
    #[must_use]
    pub fn from_mode(mode: &str) -> Self {
        if mode.eq_ignore_ascii_case("light") {
            Self::light()
        } else {
            Self::dark()
        }
    }

    #[must_use]
    pub fn dark() -> Self {
        Self {
            bg: hex(0x1a14_0e),
            surface: hex(0x241b_12),
            border: hex(0x4d3f_2a),
            text: hex(0xe8dc_c8),
            text_muted: hex(0x8e7b_5c),
            fur: hex(0xe894_5a),
            fur_dark: hex(0xf2ab_7a),
            cream: hex(0xe8dc_c8),
            ink: hex(0x1a14_0e),
            accent: hex(0x9db8_7a),
        }
    }

    #[must_use]
    pub fn light() -> Self {
        Self {
            bg: hex(0xfbf6_ef),
            surface: hex(0xffff_ff),
            border: hex(0xd9c9_ae),
            text: hex(0x2a1f_14),
            text_muted: hex(0x8a75_59),
            fur: hex(0xc263_2b),
            fur_dark: hex(0xa551_1f),
            cream: hex(0xfbf6_ef),
            ink: hex(0x2a1f_14),
            accent: hex(0x6b8e_4e),
        }
    }
}

/// 把 `0xRRGGBB` 转成 `Color32`。
fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
    )
}
