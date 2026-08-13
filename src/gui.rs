//! egui 桌宠窗口：透明无边框置顶窗，狐狸由 egui 形状绘制。
//!
//! 本模块在独立的 `std::thread` 中运行（`eframe::run_native`），与 host 的
//! Tokio runtime 解耦，通过 [`crate::channels`] 的两条单向通道通信。

use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, Rounding, FontId, Pos2, Rect, Stroke, pos2, vec2};

use crate::channels::{GuiCommand, GuiEvent};
use crate::config::DesktopPetConfig;
use crate::theme::Theme;
use crate::vitality::{VitalityStage, VitalityState};

/// 窗口宽度（逻辑点）。
const WINDOW_W: f32 = 260.0;
/// 窗口高度（逻辑点）。
const WINDOW_H: f32 = 320.0;
/// 狐狸头部半径。
const HEAD_R: f32 = 38.0;

/// 桌宠窗口入口：由 [`crate::plugin`] 在 `std::thread` 中调用。
pub fn run_gui(
    rx_cmd: Receiver<GuiCommand>,
    tx_event: Sender<GuiEvent>,
    config: DesktopPetConfig,
) {
    let theme = Theme::from_mode(&config.theme_mode);
    let opacity = config.opacity.clamp(0.1, 1.0);
    let title = config.pet_name.clone();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_W, WINDOW_H])
            .with_min_inner_size([WINDOW_W, WINDOW_H])
            .with_transparent(true)
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false),
        ..Default::default()
    };

    let result = eframe::run_native(
        &title,
        native_options,
        Box::new(move |cc| {
            let app = PetApp::new(cc, rx_cmd, tx_event, config, theme, opacity);
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    );

    if let Err(error) = result {
        eprintln!("桌宠 GUI 线程退出：{error}");
    }
}

/// 一条聊天记录。
struct ChatLine {
    role: String,
    text: String,
}

/// 头顶气泡。
struct Dialog {
    text: String,
    shown_at: Instant,
}

/// 桌宠应用状态。
struct PetApp {
    rx_cmd: Receiver<GuiCommand>,
    tx_event: Sender<GuiEvent>,
    config: DesktopPetConfig,
    theme: Theme,
    opacity: f32,
    vitality: VitalityState,
    dialog: Option<Dialog>,
    chat: Vec<ChatLine>,
    input: String,
    positioned: bool,
    last_pet: Option<Instant>,
    start: Instant,
}

impl PetApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        rx_cmd: Receiver<GuiCommand>,
        tx_event: Sender<GuiEvent>,
        config: DesktopPetConfig,
        theme: Theme,
        opacity: f32,
    ) -> Self {
        configure_visuals(cc, theme);
        Self {
            rx_cmd,
            tx_event,
            config,
            theme,
            opacity,
            vitality: VitalityState::default(),
            dialog: None,
            chat: Vec::new(),
            input: String::new(),
            positioned: false,
            last_pet: None,
            start: Instant::now(),
        }
    }

    fn on_pet_clicked(&mut self) {
        self.last_pet = Some(Instant::now());
        let _ = self.tx_event.send(GuiEvent::Petted);
    }

    fn submit_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.input.clear();
        self.chat.push(ChatLine {
            role: "user".to_string(),
            text: text.clone(),
        });
        let _ = self.tx_event.send(GuiEvent::UserMessage(text));
    }

    fn draw_dialog(&self, painter: &egui::Painter) {
        let Some(dialog) = &self.dialog else {
            return;
        };
        let font = FontId::proportional(13.0);
        let max_w = WINDOW_W - 24.0;
        let galley = painter.layout(dialog.text.clone(), font, self.theme.text, max_w);
        let pad = 8.0;
        let size = galley.size();
        let bubble = Rect::from_min_size(
            pos2((WINDOW_W - (size.x + pad * 2.0)) * 0.5, 6.0),
            vec2(size.x + pad * 2.0, size.y + pad * 2.0),
        );
        painter.rect_filled(bubble, Rounding::same(10), fade(self.theme.surface, self.opacity));
        painter.rect_stroke(bubble, Rounding::same(10), Stroke::new(1.0, self.theme.border));
        painter.galley(
            pos2(bubble.min.x + pad, bubble.min.y + pad),
            galley,
            self.theme.text,
        );
    }
}

impl eframe::App for PetApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── 1. 处理异步侧命令 ─────────────────────────────────────────
        while let Ok(cmd) = self.rx_cmd.try_recv() {
            match cmd {
                GuiCommand::ShowDialog(text) => {
                    self.dialog = Some(Dialog {
                        text,
                        shown_at: Instant::now(),
                    });
                }
                GuiCommand::AppendChat { role, text } => {
                    self.chat.push(ChatLine { role, text });
                }
                GuiCommand::SetVitality(state) => self.vitality = state,
                GuiCommand::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
        }

        // ── 2. 首次定位到屏幕右下角 ──────────────────────────────────
        if !self.positioned {
            let screen = ctx.screen_rect();
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(
                screen.right() - WINDOW_W - 24.0,
                screen.bottom() - WINDOW_H - 24.0,
            )));
            self.positioned = true;
        }

        // ── 3. 气泡自动隐藏 ──────────────────────────────────────────
        if let Some(dialog) = &self.dialog {
            let hide = self.config.dialog_auto_hide_sec > 0.0
                && dialog.shown_at.elapsed().as_secs_f32() > self.config.dialog_auto_hide_sec;
            if hide {
                self.dialog = None;
            }
        }

        let t = self.start.elapsed().as_secs_f32();
        let head = pos2(WINDOW_W * 0.5, 128.0);

        // ── 4. 画狐狸 + 气泡 + 拖拽/点击交互 ──────────────────────────
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Color32::TRANSPARENT))
            .show(ctx, |ui| {
                let painter = ui.painter();
                draw_fox(painter, head, HEAD_R, self.vitality.stage, t, &self.theme);
                self.draw_dialog(painter);

                // 狐狸区域响应拖拽（移动窗口）与点击（抚摸）
                let pet_rect = Rect::from_center_size(head, vec2(WINDOW_W, 190.0));
                let response = ui.interact(
                    pet_rect,
                    egui::Id::new("pet_body"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.clicked() {
                    self.on_pet_clicked();
                }

                // 抚摸反馈：短暂爱心
                if let Some(at) = self.last_pet {
                    if at.elapsed() < Duration::from_millis(800) {
                        let p = pos2(head.x + 28.0, head.y - 24.0);
                        painter.text(
                            p,
                            Align2::CENTER_CENTER,
                            "♥",
                            FontId::proportional(20.0),
                            self.theme.fur,
                        );
                    }
                }
            });

        // ── 5. 聊天历史（可选）与输入框 ──────────────────────────────
        self.draw_chat(ctx);

        // ── 6. 动画持续刷新 ──────────────────────────────────────────
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

impl PetApp {
    fn draw_chat(&mut self, ctx: &egui::Context) {
        if self.config.show_chat_messages {
            egui::Area::new(egui::Id::new("pet_chat"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos2(8.0, 214.0))
                .show(ctx, |ui| {
                    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(WINDOW_W - 16.0, 68.0));
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(
                        rect,
                        Rounding::same(8),
                        fade(self.theme.surface, self.opacity),
                    );
                    painter.rect_stroke(
                        rect,
                        Rounding::same(8),
                        Stroke::new(1.0, self.theme.border),
                    );
                    let mut inner = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect.shrink(6.0)),
                    );
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(&mut inner, |ui| {
                            ui.set_width(WINDOW_W - 28.0);
                            for line in &self.chat {
                                let (color, prefix) = if line.role == "user" {
                                    (self.theme.accent, "你")
                                } else {
                                    (self.theme.text, self.config.pet_name.as_str())
                                };
                                ui.label(
                                    egui::RichText::new(format!("{prefix}: {}", line.text))
                                        .color(color)
                                        .size(12.0),
                                );
                            }
                        });
                });
        }

        egui::Area::new(egui::Id::new("pet_input"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(8.0, WINDOW_H - 32.0))
            .show(ctx, |ui| {
                let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(WINDOW_W - 16.0, 24.0));
                let painter = ui.painter_at(rect);
                painter.rect_filled(
                    rect,
                    Rounding::same(8),
                    fade(self.theme.surface, self.opacity),
                );
                painter.rect_stroke(
                    rect,
                    Rounding::same(8),
                    Stroke::new(1.0, self.theme.border),
                );
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(4.0)));
                inner.add(
                    egui::TextEdit::singleline(&mut self.input)
                        .hint_text("和桌宠说点什么…")
                        .desired_width(WINDOW_W - 24.0),
                );
                let submit = inner.input(|i| i.key_pressed(egui::Key::Enter));
                if submit {
                    self.submit_input();
                }
            });
    }
}

/// 按 kitsuneflora 主题配置全局 visuals（透明背景 + 暖狐毛配色）。
fn configure_visuals(cc: &eframe::CreationContext<'_>, theme: Theme) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::TRANSPARENT;
    visuals.window_fill = theme.surface;
    visuals.extreme_bg_color = theme.surface;
    visuals.override_text_color = Some(theme.text);
    visuals.widgets.inactive.bg_fill = theme.surface;
    visuals.widgets.hovered.bg_fill = theme.surface;
    visuals.widgets.active.bg_fill = theme.surface;
    visuals.widgets.inactive.fg_stroke.color = theme.text;
    visuals.widgets.hovered.fg_stroke.color = theme.text;
    visuals.widgets.active.fg_stroke.color = theme.text;
    visuals.selection.bg_fill = theme.fur;
    cc.egui_ctx.set_visuals(visuals);
}

/// 由 egui 形状绘制一只狐狸，动作随活力值状态机变化。
fn draw_fox(painter: &egui::Painter, head: Pos2, r: f32, stage: VitalityStage, t: f32, theme: &Theme) {
    let bounce = match stage {
        VitalityStage::Energetic => (t * 5.0).sin().abs() * r * 0.18,
        VitalityStage::Active => (t * 2.0).sin() * r * 0.06,
        VitalityStage::Drowsy => -r * 0.04,
        _ => 0.0,
    };
    let head = pos2(head.x, head.y - bounce);

    // 尾巴（身体后方）
    painter.add(egui::Shape::ellipse_filled(
        pos2(head.x + r * 0.98, head.y + r * 0.55),
        vec2(r * 0.46, r * 0.30),
        theme.fur_dark,
    ));

    // 身体
    painter.add(egui::Shape::ellipse_filled(
        pos2(head.x, head.y + r * 1.35),
        vec2(r * 0.82, r * 0.95),
        theme.fur,
    ));

    // 耳朵（外 + 内）
    let left_ear = vec![
        pos2(head.x - r * 0.75, head.y - r * 0.45),
        pos2(head.x - r * 0.10, head.y - r * 0.85),
        pos2(head.x - r * 0.72, head.y - r * 1.45),
    ];
    let right_ear = vec![
        pos2(head.x + r * 0.10, head.y - r * 0.85),
        pos2(head.x + r * 0.75, head.y - r * 0.45),
        pos2(head.x + r * 0.72, head.y - r * 1.45),
    ];
    painter.add(egui::Shape::convex_polygon(left_ear.clone(), theme.fur, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(right_ear.clone(), theme.fur, Stroke::NONE));

    let left_inner = vec![
        pos2(head.x - r * 0.55, head.y - r * 0.48),
        pos2(head.x - r * 0.22, head.y - r * 0.75),
        pos2(head.x - r * 0.55, head.y - r * 1.15),
    ];
    let right_inner = vec![
        pos2(head.x + r * 0.22, head.y - r * 0.75),
        pos2(head.x + r * 0.55, head.y - r * 0.48),
        pos2(head.x + r * 0.55, head.y - r * 1.15),
    ];
    painter.add(egui::Shape::convex_polygon(left_inner, theme.cream, Stroke::NONE));
    painter.add(egui::Shape::convex_polygon(right_inner, theme.cream, Stroke::NONE));

    // 头
    painter.circle_filled(head, r, theme.fur);

    // 口鼻（奶油色）
    painter.add(egui::Shape::ellipse_filled(
        pos2(head.x, head.y + r * 0.32),
        vec2(r * 0.55, r * 0.40),
        theme.cream,
    ));

    // 眼睛
    let eye_dx = r * 0.35;
    let eye_y = head.y - r * 0.12;
    match stage {
        VitalityStage::Sleeping => {
            draw_closed_eye(painter, pos2(head.x - eye_dx, eye_y), r, theme.ink);
            draw_closed_eye(painter, pos2(head.x + eye_dx, eye_y), r, theme.ink);
        }
        VitalityStage::Drowsy => {
            draw_slit_eye(painter, pos2(head.x - eye_dx, eye_y), r, theme.ink);
            draw_slit_eye(painter, pos2(head.x + eye_dx, eye_y), r, theme.ink);
        }
        _ => {
            let eye_r = r * 0.10;
            painter.circle_filled(pos2(head.x - eye_dx, eye_y), eye_r, theme.ink);
            painter.circle_filled(pos2(head.x + eye_dx, eye_y), eye_r, theme.ink);
            let glint = eye_r * 0.35;
            painter.circle_filled(
                pos2(head.x - eye_dx + eye_r * 0.3, eye_y - eye_r * 0.3),
                glint,
                Color32::WHITE,
            );
            painter.circle_filled(
                pos2(head.x + eye_dx + eye_r * 0.3, eye_y - eye_r * 0.3),
                glint,
                Color32::WHITE,
            );
        }
    }

    // 鼻头
    painter.circle_filled(pos2(head.x, head.y + r * 0.10), r * 0.09, theme.ink);

    // 腮红
    let blush = fade(theme.accent, 0.55);
    painter.circle_filled(pos2(head.x - r * 0.62, head.y + r * 0.16), r * 0.14, blush);
    painter.circle_filled(pos2(head.x + r * 0.62, head.y + r * 0.16), r * 0.14, blush);

    // 睡眠 Zzz
    if stage == VitalityStage::Sleeping {
        let phase = (t * 0.8).fract();
        for (i, ch) in ["z", "z", "z"].iter().enumerate() {
            let a = (phase + i as f32 * 0.33).fract();
            let p = pos2(
                head.x + r * 0.7 + i as f32 * r * 0.28,
                head.y - r * 1.15 - a * r * 0.8,
            );
            painter.text(
                p,
                Align2::LEFT_BOTTOM,
                *ch,
                FontId::proportional(10.0 + i as f32 * 4.0),
                theme.text_muted,
            );
        }
    }
}

fn draw_closed_eye(painter: &egui::Painter, center: Pos2, r: f32, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    let left = pos2(center.x - r * 0.5, center.y);
    let right = pos2(center.x + r * 0.5, center.y);
    let mid = pos2(center.x, center.y + r * 0.35);
    painter.line_segment([left, mid], stroke);
    painter.line_segment([mid, right], stroke);
}

fn draw_slit_eye(painter: &egui::Painter, center: Pos2, r: f32, color: Color32) {
    let stroke = Stroke::new(2.0, color);
    painter.line_segment(
        [pos2(center.x - r * 0.5, center.y), pos2(center.x + r * 0.5, center.y)],
        stroke,
    );
}

/// 对颜色施加窗口透明度。
fn fade(color: Color32, opacity: f32) -> Color32 {
    let o = opacity.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (o * 255.0) as u8)
}
