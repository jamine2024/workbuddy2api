#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod native_hud;
mod native_window;
mod live_hud;
mod net;
mod sys;
mod tray;
mod usage_charts;
mod model_rates;
mod login_browser;

#[cfg(test)]
mod ui_tests;

use eframe::egui::{self, Color32, FontId, RichText, ScrollArea, ViewportBuilder, ViewportCommand};
use serde_json::Value;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tray::{Action as TrayAct, DesktopTray, State as TrayState};
use tray_icon::Icon;

// 桌面工作台：允许横向缩放，紧凑窗口自动改为单列并保留页面滚动。
pub const WIN_W: f32 = 1040.0;
const WIN_H: f32 = 740.0;
const NAV_W: f32 = 168.0;
const HUD_W: f32 = 252.0;
const HUD_H: f32 = 112.0;
const HUD_GUTTER: f32 = 6.0;
const HUD_RADIUS: f32 = 16.0;

// ── 现代化配色 ──
const BG: Color32 = Color32::from_rgb(0x14, 0x15, 0x18);
const PANEL: Color32 = Color32::from_rgb(0x19, 0x1A, 0x1E);
const CARD: Color32 = Color32::from_rgb(0x1D, 0x1E, 0x23);
const CARD2: Color32 = Color32::from_rgb(0x26, 0x27, 0x2E);
const LINE: Color32 = Color32::from_rgb(0x30, 0x31, 0x39);
const TX: Color32 = Color32::from_rgb(0xEC, 0xED, 0xF1);
const TX2: Color32 = Color32::from_rgb(0xB0, 0xB2, 0xBD);
const TX3: Color32 = Color32::from_rgb(0x8B, 0x8E, 0x9C);
const ACC: Color32 = Color32::from_rgb(0xA0, 0xAC, 0xFF);
const ACC2: Color32 = Color32::from_rgb(0xB5, 0xA2, 0xDD);
const INK: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x32);
const OK: Color32 = Color32::from_rgb(0x7E, 0xC4, 0x9E);
const WARN: Color32 = Color32::from_rgb(0xD9, 0xB6, 0x73);
const ERR: Color32 = Color32::from_rgb(0xE0, 0x8D, 0x95);
const SEL: Color32 = Color32::from_rgb(0x2A, 0x2D, 0x40);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Nav {
    Overview,
    AddAcct,
    Usage,
    Models,
    Logs,
    About,
    Settings,
}

impl Nav {
    fn label(&self) -> &'static str {
        match self {
            Nav::Overview => "总览",
            Nav::AddAcct => "添加账号",
            Nav::Usage => "用量统计",
            Nav::Models => "模型管理",
            Nav::Logs => "运行日志",
            Nav::About => "关于",
            Nav::Settings => "设置",
        }
    }
    const ALL: [Nav; 7] = [
        Nav::Overview,
        Nav::AddAcct,
        Nav::Usage,
        Nav::Models,
        Nav::Logs,
        Nav::About,
        Nav::Settings,
    ];
}

enum Msg {
    State(sys::Collected),
    Toast(String, bool),
    Busy(bool, String),
    Models(u64, Result<Vec<net::ModelRow>, String>, Result<Value, String>, bool),
    RefreshModels,
    OAuthLog(String, bool),
    OAuthUrl(String, String, Option<login_browser::Profile>),
    LoginBrowserOpened(String, Result<(), String>),
    LoginExchangeFailed(String),
    OAuthSaved(net::SavedAccount),
    Usage(sys::UsageStats),
    Status(Value),
    Tray(TrayAct),
}

struct App {
    native: native_window::MainWindow,
    nav: Nav,
    st: sys::Collected,
    busy: bool,
    busy_text: String,
    toast: Option<(String, bool, Instant)>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    port_str: String,
    key_str: String,
    key_visible: bool,
    autostart_gw: bool,
    autostart_ui: bool,
    lan: String,
    models: Vec<net::ModelRow>,
    model_refresh_id: u64,
    model_loading: bool,
    model_error: Option<String>,
    gateway_status_error: Option<String>,
    free_gateways: [bool; 2],
    saved_free_gateways: [bool; 2],
    realm: usize,
    auth_url: String,
    auth_state: Option<String>,
    login_profile: Option<login_browser::Profile>,
    login_target_uid: String,
    login_browser_opened: bool,
    oauth_log: Vec<(String, bool)>,
    acct_busy: bool,
    pending_del: Option<String>,
    usage: Option<sys::UsageStats>,
    live: usage_charts::LiveHistory,
    status: Option<Value>,
    // 托盘
    tray: Option<DesktopTray>,
    quit: Arc<AtomicBool>,
    start_hidden: bool,
    // Token 悬浮窗（独立原生窗口，可拖出主窗口）
    float_open: Arc<AtomicBool>,
    float_stats: Arc<Mutex<live_hud::HudStats>>,
}

// ────────────────────────── 小工具 ──────────────────────────

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn copy(text: &str) -> bool {
    match arboard::Clipboard::new() {
        Ok(mut c) => c.set_text(text.to_string()).is_ok(),
        Err(_) => false,
    }
}

fn btn(ui: &mut egui::Ui, label: &str, fill: Color32, fg: Color32, enabled: bool) -> bool {
    let enabled = enabled && ui.is_enabled();
    let primary = fill == ACC || fill == OK || fill == ERR;
    let fg = if !enabled {
        TX3
    } else if primary {
        INK
    } else {
        fg
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.into(), FontId::proportional(12.5), fg);
    let size = egui::vec2(galley.size().x + 28.0, 34.0);
    let (id, space) = ui.allocate_space(size);
    let rect = ui.layout().align_size_within_rect(size, space);
    let response = ui.interact(
        rect,
        id,
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    let background = if !enabled {
        CARD2.gamma_multiply(0.55)
    } else if response.is_pointer_button_down_on() {
        fill.gamma_multiply(0.8)
    } else if response.hovered() {
        if primary {
            fill.linear_multiply(1.08)
        } else {
            Color32::from_rgb(49, 51, 60)
        }
    } else {
        fill
    };
    ui.painter().rect_filled(rect, 7.0, background);
    ui.painter()
        .galley(rect.center() - galley.size() * 0.5, galley, fg);
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect.expand(2.0), 9.0, egui::Stroke::new(1.0, ACC));
    }
    response
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        })
        .clicked()
        && enabled
}

fn copy_action(ui: &mut egui::Ui, label: &str, enabled: bool) -> bool {
    let enabled = enabled && ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(26.0, 26.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    if response.hovered() && enabled {
        ui.painter().rect_filled(rect, 5.0, CARD2);
    }
    let color = if enabled {
        TX2
    } else {
        TX3.gamma_multiply(0.5)
    };
    let stroke = egui::Stroke::new(1.2, color);
    let front = egui::Rect::from_min_size(rect.min + egui::vec2(10.0, 9.0), egui::vec2(9.0, 11.0));
    ui.painter().rect_stroke(front, 1.8, stroke);
    ui.painter().line_segment(
        [
            rect.min + egui::vec2(7.0, 17.0),
            rect.min + egui::vec2(7.0, 6.0),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            rect.min + egui::vec2(7.0, 6.0),
            rect.min + egui::vec2(15.0, 6.0),
        ],
        stroke,
    );
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect, 5.0, egui::Stroke::new(1.0, ACC));
    }
    response
        .on_hover_text(label)
        .on_hover_cursor(if enabled {
            egui::CursorIcon::PointingHand
        } else {
            egui::CursorIcon::Default
        })
        .clicked()
        && enabled
}

fn card(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(egui::Stroke::NONE)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).color(TX).size(13.5).strong());
                ui.add_space(10.0);
            }
            body(ui);
        });
}

/// 终端日志使用独立深色底，便于扫描长行。
fn term_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(Color32::from_rgb(0x08, 0x0A, 0x0F))
        .stroke(egui::Stroke::NONE)
        .rounding(egui::Rounding::same(9.0))
        .inner_margin(egui::Margin::same(14.0))
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.09))
        .rounding(egui::Rounding::same(5.0))
        .inner_margin(egui::Margin::symmetric(7.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(11.5));
        });
}

fn page_head(ui: &mut egui::Ui, title: &str, sub: &str) {
    ui.label(RichText::new(title).color(TX).size(22.0).strong());
    ui.add_space(4.0);
    ui.label(RichText::new(sub).color(TX3).size(12.5));
    ui.add_space(20.0);
}

fn stat_tile(ui: &mut egui::Ui, label: &str, value: &str, color: Color32, unit: &str) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(egui::Stroke::NONE)
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(label).color(TX3).size(12.0));
            ui.add_space(5.0);
            ui.label(RichText::new(value).color(color).size(23.0).strong());
            ui.add_space(2.0);
            ui.label(RichText::new(unit).color(TX3).size(11.5));
        });
}

fn toggle(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(38.0, 22.0), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
    });
    let progress = ui.ctx().animate_bool_with_time(response.id, *value, 0.14);
    let fill = if *value {
        ACC
    } else if response.hovered() {
        Color32::from_gray(76)
    } else {
        Color32::from_gray(59)
    };
    ui.painter().rect_filled(rect, 11.0, fill);
    let center = egui::pos2(rect.left() + 11.0 + progress * 16.0, rect.center().y);
    ui.painter().circle_filled(
        center,
        8.0,
        if *value { INK } else { Color32::from_gray(211) },
    );
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect.expand(3.0), 13.0, egui::Stroke::new(1.0, ACC));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn compact_hud_number(value: f64) -> String {
    for (limit, suffix) in [
        (1e18, "E"),
        (1e15, "P"),
        (1e12, "T"),
        (1e9, "B"),
        (1e6, "M"),
        (1e3, "K"),
    ] {
        if value >= limit {
            let number = format!("{:.2}", value / limit);
            return format!(
                "{}{suffix}",
                number.trim_end_matches('0').trim_end_matches('.')
            );
        }
    }
    format!("{value:.0}")
}

fn hud_viewport() -> ViewportBuilder {
    ViewportBuilder::default()
        .with_title(native_hud::TITLE)
        .with_inner_size([HUD_W, HUD_H])
        .with_min_inner_size([HUD_W, HUD_H])
        .with_decorations(false)
        .with_transparent(!cfg!(target_os = "windows"))
        .with_always_on_top()
        .with_active(false)
        .with_resizable(false)
        .with_taskbar(false)
}

/// Windows clips the native window to the rounded surface; the backing paint is fully opaque.
/// Returns the close hit area for interaction checks.
fn token_hud(ui: &mut egui::Ui, open: &AtomicBool, stats: &live_hud::HudStats) -> egui::Rect {
    let (viewport, _) = ui.allocate_exact_size(egui::vec2(HUD_W, HUD_H), egui::Sense::hover());
    let surface = viewport.shrink(HUD_GUTTER);
    let p = ui.painter();
    if cfg!(target_os = "windows") {
        // Do not leave alpha or antialiased dark pixels inside the native window region.
        p.rect_filled(viewport, 0.0, CARD);
    } else {
        p.rect_filled(surface, HUD_RADIUS, CARD);
    }
    stats.paint_background(p, surface);

    let close = egui::Rect::from_min_size(
        surface.right_top() + egui::vec2(-31.0, 7.0),
        egui::vec2(24.0, 24.0),
    );
    // Separate hit areas prevent a close click from ever starting a native drag.
    for (id, rect) in [
        (
            "header",
            egui::Rect::from_min_max(surface.min, egui::pos2(close.left(), close.bottom())),
        ),
        (
            "body",
            egui::Rect::from_min_max(egui::pos2(surface.left(), close.bottom()), surface.max),
        ),
    ] {
        let drag = ui.interact(rect, ui.id().with(id), egui::Sense::click_and_drag());
        if drag.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        drag.on_hover_cursor(egui::CursorIcon::Grab);
    }

    // Small, static throughput mark; no decorative or simulated chart data.
    let origin = surface.min + egui::vec2(16.0, 13.0);
    for (i, height) in [4.0, 10.0, 7.0].into_iter().enumerate() {
        p.rect_filled(
            egui::Rect::from_min_size(
                origin + egui::vec2(i as f32 * 4.0, 11.0 - height),
                egui::vec2(2.0, height),
            ),
            1.0,
            ACC,
        );
    }
    p.text(
        surface.min + egui::vec2(33.0, 19.0),
        egui::Align2::LEFT_CENTER,
        "Token 监测",
        FontId::proportional(10.5),
        TX3,
    );

    let tps = if stats.tps.is_finite() { stats.tps.max(0.0) } else { 0.0 };
    let speed = if tps < 10_000.0 {
        format!("{tps:.0}")
    } else {
        compact_hud_number(tps as f64)
    };
    let mut speed = p.layout_no_wrap(speed, FontId::proportional(30.0), TX);
    if speed.size().x > 84.0 {
        speed = p.layout_no_wrap(
            speed.text().into(),
            FontId::proportional(30.0 * 84.0 / speed.size().x),
            TX,
        );
    }
    let speed_pos = surface.min + egui::vec2(16.0, 34.0);
    let unit_pos = speed_pos + egui::vec2(speed.size().x + 5.0, 27.0);
    p.galley(speed_pos, speed, TX);
    p.text(
        unit_pos,
        egui::Align2::LEFT_BOTTOM,
        "tok/s",
        FontId::proportional(10.0),
        TX3,
    );
    p.text(
        surface.min + egui::vec2(16.0, 80.0),
        egui::Align2::LEFT_CENTER,
        "实时速度",
        FontId::proportional(10.5),
        TX3,
    );

    let total_text = format!("{}{}", if stats.estimated { "≈" } else { "" }, fmt_num(stats.total));
    let mut total = p.layout_no_wrap(total_text.clone(), FontId::proportional(17.0), ACC);
    if total.size().x > 96.0 {
        total = p.layout_no_wrap(total_text, FontId::proportional(17.0 * 96.0 / total.size().x), ACC);
    }
    p.galley(surface.min + egui::vec2(224.0 - total.size().x, 54.0 - total.size().y / 2.0), total, ACC);
    p.text(
        surface.min + egui::vec2(224.0, 80.0),
        egui::Align2::RIGHT_CENTER,
        "累计 Token",
        FontId::proportional(10.5),
        TX3,
    );
    ui.interact(
        egui::Rect::from_min_max(
            surface.min + egui::vec2(137.0, 36.0),
            surface.max - egui::vec2(10.0, 8.0),
        ),
        ui.id().with("total-tooltip"),
        egui::Sense::hover(),
    )
    .on_hover_text(format!("累计 {} Token\n{}", fmt_num(stats.total), if stats.estimated {
        "生成中按输出文本估算，每秒更新；结束后以实际用量校准。"
    } else { "已完成请求的实际输出 Token。" }));
    ui.interact(
        egui::Rect::from_min_size(surface.min + egui::vec2(12.0, 34.0), egui::vec2(116.0, 53.0)),
        ui.id().with("speed-tooltip"), egui::Sense::hover(),
    ).on_hover_text(if stats.connected {
        "最近一秒收到的输出速率（估算），包含思考与工具输出；空闲或无新增输出时为 0。"
    } else { "等待网关实时统计连接；空闲时为 0。" });

    let response = ui.interact(close, ui.id().with("close-hud"), egui::Sense::click());
    if response.hovered() || response.has_focus() {
        p.rect_filled(close, 7.0, CARD2);
    }
    let stroke = egui::Stroke::new(1.3, if response.hovered() { TX } else { TX3 });
    let cross = close.shrink(8.0);
    p.line_segment([cross.left_top(), cross.right_bottom()], stroke);
    p.line_segment([cross.left_bottom(), cross.right_top()], stroke);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "隐藏悬浮窗"));
    if response
        .on_hover_text("隐藏悬浮窗 · 可从托盘重新打开")
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        open.store(false, Ordering::Relaxed);
        ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
    }
    close
}

fn setting_row(ui: &mut egui::Ui, title: &str, description: &str, value: &mut bool) -> egui::Rect {
    ui.horizontal(|ui| {
        let text_width = (ui.available_width() - 46.0).max(100.0);
        ui.allocate_ui_with_layout(
            egui::vec2(text_width, 56.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_min_width(text_width);
                ui.add_space(5.0);
                ui.label(RichText::new(title).color(TX).size(14.0));
                ui.label(RichText::new(description).color(TX3).size(12.0));
            },
        );
        toggle(ui, value, title).rect
    })
    .inner
}

/// 侧栏矢量图标。
/// 直接在给定 rect 内绘制（导航行负责分配 14px 槽位与点击命中）
fn nav_icon(painter: &egui::Painter, nav: Nav, rect: egui::Rect, selected: bool) {
    let c = if selected { ACC } else { TX3 };
    let stroke = egui::Stroke::new(1.6, c);
    let min = rect.min;
    let max = rect.max;
    let w = max.x - min.x;
    let h = max.y - min.y;
    let pad = w * 0.13;
    let left = min.x + pad;
    let right = max.x - pad;
    let top = min.y + pad;
    let bottom = max.y - pad;
    let mid_x = (min.x + max.x) * 0.5;
    let mid_y = (min.y + max.y) * 0.5;

    match nav {
        Nav::Overview => {
            // dashboard: 外框 + 内部竖条分区
            painter.rect_stroke(
                egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
                2.0,
                stroke,
            );
            let x1 = left + w * 0.22;
            let x2 = left + w * 0.45;
            let x3 = left + w * 0.68;
            painter.line_segment(
                [
                    egui::pos2(x1, top + h * 0.22),
                    egui::pos2(x1, bottom - h * 0.22),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(x2, top + h * 0.38),
                    egui::pos2(x2, bottom - h * 0.22),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(x3, top + h * 0.30),
                    egui::pos2(x3, bottom - h * 0.22),
                ],
                stroke,
            );
        }
        Nav::AddAcct => {
            // user-plus 简化：圆 + 十字
            let r = (w * 0.28).min(h * 0.28);
            painter.circle_stroke(egui::pos2(mid_x, mid_y - h * 0.08), r, stroke);
            let plus_y = mid_y + h * 0.22;
            painter.line_segment(
                [
                    egui::pos2(mid_x - w * 0.14, plus_y),
                    egui::pos2(mid_x + w * 0.14, plus_y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(mid_x, plus_y - h * 0.14),
                    egui::pos2(mid_x, plus_y + h * 0.14),
                ],
                stroke,
            );
        }
        Nav::Usage => {
            // bar-chart: 三根高度不同的竖条
            let bar_w = w * 0.16;
            let xs = [left + w * 0.22, left + w * 0.50, left + w * 0.78];
            let hs = [h * 0.45, h * 0.72, h * 0.55];
            for (i, x) in xs.iter().enumerate() {
                let half = bar_w * 0.5;
                let bar_top = bottom - hs[i];
                painter.line_segment(
                    [
                        egui::pos2(*x - half, bar_top),
                        egui::pos2(*x - half, bottom - pad * 0.5),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(*x + half, bar_top),
                        egui::pos2(*x + half, bottom - pad * 0.5),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(*x - half, bar_top),
                        egui::pos2(*x + half, bar_top),
                    ],
                    stroke,
                );
            }
        }
        Nav::Models => {
            // box / cube 简化为六边形轮廓
            let cx = mid_x;
            let cy = mid_y;
            let rx = w * 0.32;
            let ry = h * 0.26;
            let points: Vec<egui::Pos2> = (0..6)
                .map(|i| {
                    let a = std::f32::consts::PI / 3.0 * i as f32 - std::f32::consts::FRAC_PI_2;
                    egui::pos2(cx + rx * a.cos(), cy + ry * a.sin())
                })
                .collect();
            painter.add(egui::Shape::closed_line(points.clone(), stroke));
            // 内部透视线条
            painter.line_segment([points[0], points[3]], stroke);
            painter.line_segment([points[1], points[4]], stroke);
        }
        Nav::Logs => {
            // file-text: 文档 + 折角 + 横线
            let fold = h * 0.22;
            let body = [
                egui::pos2(left, top),
                egui::pos2(right - fold, top),
                egui::pos2(right, top + fold),
                egui::pos2(right, bottom),
                egui::pos2(left, bottom),
            ];
            painter.add(egui::Shape::closed_line(body.to_vec(), stroke));
            painter.line_segment(
                [
                    egui::pos2(right - fold, top),
                    egui::pos2(right - fold, top + fold),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(right - fold, top + fold),
                    egui::pos2(right, top + fold),
                ],
                stroke,
            );
            let y1 = top + h * 0.36;
            let y2 = top + h * 0.54;
            let y3 = top + h * 0.72;
            let margin = w * 0.18;
            painter.line_segment(
                [
                    egui::pos2(left + margin, y1),
                    egui::pos2(right - margin, y1),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left + margin, y2),
                    egui::pos2(right - margin, y2),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(left + margin, y3),
                    egui::pos2(right - margin * 1.2, y3),
                ],
                stroke,
            );
        }
        Nav::About => {
            // info: 圆 + i
            let r = w * 0.32;
            painter.circle_stroke(egui::pos2(mid_x, mid_y), r, stroke);
            painter.circle_filled(egui::pos2(mid_x, mid_y - h * 0.16), 1.3, c);
            painter.line_segment(
                [
                    egui::pos2(mid_x, mid_y - h * 0.02),
                    egui::pos2(mid_x, mid_y + h * 0.18),
                ],
                stroke,
            );
        }
        Nav::Settings => {
            for (fraction, knob) in [(0.0, 0.3), (0.5, 0.7), (1.0, 0.4)] {
                let y = top + (bottom - top) * fraction;
                painter.line_segment([egui::pos2(left, y), egui::pos2(right, y)], stroke);
                let point = egui::pos2(left + (right - left) * knob, y);
                painter.circle_filled(point, 1.6, if selected { SEL } else { PANEL });
                painter.circle_stroke(point, 1.6, egui::Stroke::new(1.2, c));
            }
        }
    }
}

fn setup_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};
    // egui 内置字体不含中文字形，必须挂系统字体，否则界面全是豆腐块
    for p in [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhl.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
    ] {
        if let Ok(bytes) = fs::read(p) {
            let mut fonts = FontDefinitions::default();
            let name = "cjk";
            fonts
                .font_data
                .insert(name.to_owned(), FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, name.to_owned());
            // 等宽族「追加」cjk 作兜底（push 到末尾，绝不 insert 到 0）：
            // ASCII 仍用 egui 默认等宽字体（日志/URL 列对齐不变），只有等宽字体
            // 缺失的中文字形才回退到 msyh —— 否则日志里的中文会渲染成豆腐块。
            fonts
                .families
                .entry(FontFamily::Monospace)
                .or_default()
                .push(name.to_owned());
            ctx.set_fonts(fonts);
            break;
        }
    }
}

fn setup_theme(ctx: &egui::Context) {
    // set_visuals only styles the currently selected egui theme. Pin it before
    // customizing, otherwise the first Windows Light event selects default white controls.
    ctx.set_theme(egui::Theme::Dark);
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.extreme_bg_color = BG;
    v.window_fill = CARD;
    v.window_stroke = egui::Stroke::new(1.0, LINE);
    v.override_text_color = Some(TX);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.weak_bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TX2);
    v.widgets.inactive.bg_fill = CARD2;
    v.widgets.inactive.weak_bg_fill = CARD2;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TX);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x30, 0x32, 0x3A);
    v.widgets.hovered.weak_bg_fill = CARD2;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TX);
    v.widgets.active.bg_fill = ACC;
    v.widgets.active.weak_bg_fill = SEL;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, INK);
    v.selection.bg_fill = SEL;
    v.selection.stroke = egui::Stroke::new(1.0, ACC);
    v.hyperlink_color = ACC;
    v.widgets.inactive.rounding = egui::Rounding::same(5.0);
    v.widgets.hovered.rounding = egui::Rounding::same(5.0);
    v.widgets.active.rounding = egui::Rounding::same(5.0);
    v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    v.widgets.active.bg_stroke = egui::Stroke::NONE;
    ctx.set_visuals(v);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.icon_width = 17.0;
    style.spacing.icon_spacing = 7.0;
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, FontId::proportional(11.5));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, FontId::monospace(12.0));
    ctx.all_styles_mut(|target| *target = style.clone());
}

fn make_icon_data() -> egui::IconData {
    egui::IconData {
        rgba: include_bytes!("../assets/WorkBuddy2API-128.rgba").to_vec(),
        width: 128,
        height: 128,
    }
}

fn make_tray_icon() -> Icon {
    let icon = make_icon_data();
    Icon::from_rgba(icon.rgba, icon.width, icon.height).expect("构造托盘图标")
}

// ────────────────────────── App ──────────────────────────

impl App {
    fn new(cc: &eframe::CreationContext<'_>, start_hidden: bool) -> Self {
        setup_fonts(&cc.egui_ctx);
        setup_theme(&cc.egui_ctx);
        cc.egui_ctx
            .send_viewport_cmd(ViewportCommand::SetTheme(egui::SystemTheme::Dark));

        let (tx, rx) = mpsc::channel();
        let cfg = sys::read_config();
        let port = sys::cfg_port(&cfg);

        let app = App {
            native: native_window::MainWindow::from_context(cc),
            nav: if std::env::args().any(|arg| arg == "--page=usage") {
                Nav::Usage
            } else if std::env::args().any(|arg| arg == "--page=models") {
                Nav::Models
            } else {
                Nav::Overview
            },
            st: sys::Collected {
                port,
                ..Default::default()
            },
            busy: false,
            busy_text: String::new(),
            toast: None,
            tx,
            rx,
            port_str: port.to_string(),
            key_str: sys::api_key(&cfg),
            key_visible: false,
            autostart_gw: sys::startup_on(),
            autostart_ui: sys::gui_startup_on(),
            lan: sys::lan_ip(),
            models: Vec::new(),
            model_refresh_id: 0,
            model_loading: false,
            model_error: None,
            gateway_status_error: None,
            free_gateways: sys::free_gateway_settings(&cfg),
            saved_free_gateways: sys::free_gateway_settings(&cfg),
            realm: 0,
            auth_url: String::new(),
            auth_state: None,
            login_profile: None,
            login_target_uid: String::new(),
            login_browser_opened: false,
            oauth_log: Vec::new(),
            acct_busy: false,
            pending_del: None,
            usage: None,
            live: usage_charts::LiveHistory::default(),
            status: None,
            tray: None,
            quit: Arc::new(AtomicBool::new(false)),
            start_hidden,
            float_open: Arc::new(AtomicBool::new(std::env::args().any(|arg| arg == "--hud"))),
            float_stats: Arc::new(Mutex::new(live_hud::HudStats::default())),
        };
        app.native.dark_title_bar();

        // Standalone copies extract the embedded gateway into the user's data directory.
        std::thread::spawn(|| {
            let _ = sys::ensure_gateway();
            // Repair existing startup shortcuts after moving from a desktop/portable copy.
            if sys::startup_on() { let _ = sys::set_startup(true); }
            if sys::gui_startup_on() { let _ = sys::set_gui_startup(true); }
        });

        // 后台心跳：一切阻塞 IO 都在这条线程，主线程只渲染
        let tx2 = app.tx.clone();
        std::thread::spawn(move || {
            loop {
                let _ = tx2.send(Msg::State(sys::collect()));
                std::thread::sleep(Duration::from_secs(3));
            }
        });

        // Sample real log totals once per second, even when the usage page is not open.
        // Unchanged files reuse parsed totals; idle sampling therefore does no full log reads.
        let metrics_tx = app.tx.clone();
        let metrics_ctx = cc.egui_ctx.clone();
        let metrics_stats = app.float_stats.clone();
        let metrics_window = app.native;
        std::thread::spawn(move || {
            let mut signature = None;
            let mut cached = sys::UsageStats::default();
            let mut hud = live_hud::HudStats::default();
            let started = Instant::now();
            loop {
                let tick = Instant::now();
                if native_window::quit_requested() {
                    let _ = metrics_tx.send(Msg::Tray(TrayAct::Quit));
                    metrics_ctx.request_repaint();
                    metrics_window.wake();
                    // Keep waking hidden viewports until eframe has processed the Close command.
                }
                let cfg = sys::read_config();
                let live = net::fetch_live_usage(sys::cfg_port(&cfg), &sys::api_key(&cfg)).ok();
                let current = fs::metadata(sys::gw_log())
                    .ok()
                    .map(|m| (m.len(), m.modified().ok()));
                if current != signature {
                    cached = sys::parse_usage();
                    signature = current;
                }
                if metrics_tx.send(Msg::Usage(cached.clone())).is_err() {
                    break;
                }
                hud.sample(started.elapsed().as_secs_f64(), &cached, live.as_ref());
                // Keep the independent HUD current even while the main window is hidden.
                if let Ok(mut stats) = metrics_stats.lock() { *stats = hud.clone(); }
                metrics_ctx.request_repaint_of(egui::ViewportId::ROOT);
                metrics_window.wake();
                metrics_ctx.request_repaint_of(egui::ViewportId::from_hash_of("token_hud"));
                std::thread::sleep(Duration::from_secs(1).saturating_sub(tick.elapsed()));
            }
        });
        app
    }

    fn setup_tray(&mut self, ctx: &egui::Context) {
        let context = ctx.clone();
        let window = self.native;
        let tx = self.tx.clone();
        self.tray = Some(DesktopTray::new(move |action| {
            let _ = tx.send(Msg::Tray(action));
            context.request_repaint_of(egui::ViewportId::ROOT);
            window.wake();
        }));
    }

    fn handle(&mut self, m: Msg, ctx: &egui::Context) {
        match m {
            Msg::State(c) => self.st = c,
            Msg::Toast(t, ok) => self.toast = Some((t, ok, Instant::now())),
            Msg::Busy(b, t) => {
                self.busy = b;
                self.busy_text = t;
            }
            Msg::Models(id, result, status, silent) => {
                if id != self.model_refresh_id { return; }
                self.model_loading = false;
                match status {
                    Ok(value) => {
                        self.status = Some(value);
                        self.gateway_status_error = None;
                    }
                    Err(error) => {
                        self.status = None;
                        self.gateway_status_error = Some(error);
                    }
                }
                match result {
                    Ok(list) => {
                        self.models = list;
                        self.model_error = None;
                        if !silent {
                            let failures = self.free_gateway_failures();
                            let (message, ok) = if !failures.is_empty() {
                                (format!("已更新 {} 个模型；{}，请查看免费网关状态", self.models.len(), failures.join("、")), false)
                            } else if self.gateway_status_error.is_some() || self.free_gateway_status("opencode").is_none()
                                || self.free_gateway_status("kilo").is_none() {
                                (format!("已更新 {} 个模型；免费网关状态未确认", self.models.len()), false)
                            } else {
                                (format!("已更新 {} 个模型目录，调用结果以上游响应为准", self.models.len()), true)
                            };
                            self.toast = Some((message, ok, Instant::now()));
                        }
                    }
                    Err(error) => {
                        self.models.clear();
                        self.model_error = Some(error.clone());
                        if !silent {
                            self.toast = Some((format!("获取模型失败: {error}"), false, Instant::now()));
                        }
                    }
                }
            }
            Msg::RefreshModels => self.model_refresh(true),
            Msg::OAuthLog(s, ok) => self.oauth_log.push((s, ok)),
            Msg::OAuthUrl(url, state, profile) => {
                self.acct_busy = false;
                self.login_profile = profile;
                self.login_browser_opened = false;
                if state.is_empty() {
                    self.auth_state = None;
                    self.auth_url.clear();
                } else {
                    self.auth_url = url;
                    self.auth_state = Some(state);
                    self.oauth_log
                        .push(("已准备独立登录环境，请打开内置指纹浏览器完成登录。".into(), true));
                }
            }
            Msg::LoginBrowserOpened(id, result) => {
                if self.login_profile.as_ref().is_some_and(|profile| profile.id == id) {
                    self.acct_busy = false;
                    self.login_browser_opened = result.is_ok();
                    match result {
                        Ok(()) => self.oauth_log.push(("独立指纹浏览器已打开，完成授权后点击「我已完成登录」。".into(), true)),
                        Err(error) => self.oauth_log.push((error, false)),
                    }
                }
            }
            Msg::LoginExchangeFailed(error) => {
                self.acct_busy = false;
                self.oauth_log.push((format!("登录未完成: {error}"), false));
            }
            Msg::OAuthSaved(a) => {
                self.acct_busy = false;
                self.auth_state = None;
                self.auth_url.clear();
                self.login_profile = None;
                self.login_browser_opened = false;
                self.oauth_log.push((
                    format!(
                        "已保存 {} （UID {}，昵称 {}）",
                        a.file,
                        a.uid,
                        if a.nick.is_empty() { "-" } else { &a.nick }
                    ),
                    true,
                ));
                if !a.checkin.is_empty() {
                    self.oauth_log
                        .push((format!("每日签到: {}", a.checkin), true));
                }
                self.oauth_log
                    .push(("正在自动重启网关使账号生效…".into(), true));
                self.refresh();
                if !self.busy {
                    self.restart(true);
                }
            }
            Msg::Usage(u) => {
                self.live.record(&u);
                self.usage = Some(u);
            }
            Msg::Status(v) => {
                self.status = Some(v);
                self.gateway_status_error = None;
            }
            Msg::Tray(a) => match a {
                TrayAct::Show => {
                    for command in [
                        ViewportCommand::Minimized(false),
                        ViewportCommand::Visible(true),
                        ViewportCommand::Focus,
                    ] {
                        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, command);
                    }
                }
                TrayAct::ToggleHud => {
                    self.float_open.fetch_xor(true, Ordering::Relaxed);
                }
                TrayAct::Quit => {
                    self.quit.store(true, Ordering::SeqCst);
                    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Close);
                }
                TrayAct::Start if !self.busy && !self.st.running => self.start(),
                TrayAct::Stop if !self.busy && self.st.running => self.stop(),
                TrayAct::Error(error) => {
                    self.toast =
                        Some((format!("托盘初始化失败：{}", error), false, Instant::now()));
                }
                _ => {}
            },
        }
    }

    fn refresh(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::State(sys::collect()));
        });
    }

    fn usage_refresh(&self) {
        let tx = self.tx.clone();
        let port = self.st.port;
        let key = sys::api_key(&sys::read_config());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Usage(sys::parse_usage()));
            if let Ok(v) = net::fetch_status(port, &key) {
                let _ = tx.send(Msg::Status(v));
            }
        });
    }

    fn toggle(&mut self) {
        if self.busy {
            return;
        }
        if self.st.running {
            self.stop();
        } else {
            self.start();
        }
    }

    fn start(&mut self) {
        if self.busy {
            return;
        }
        if sys::find_exe().is_none() {
            self.toast = Some((
                "找不到 wb2api.exe，请把本 exe 放在项目根目录".into(),
                false,
                Instant::now(),
            ));
            return;
        }
        self.busy = true;
        self.busy_text = "启动中…".into();
        self.reset_model_snapshot();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let started = match sys::start_gateway() {
                Ok((pid, ok)) => {
                    let msg = if ok {
                        format!("启动成功（PID {}）", pid)
                    } else {
                        format!("已拉起（PID {}），探活未通过，请查看运行日志", pid)
                    };
                    let _ = tx.send(Msg::Toast(msg, ok));
                    true
                }
                Err(e) => {
                    let _ = tx.send(Msg::Toast(format!("启动失败: {}", e), false));
                    false
                }
            };
            let _ = tx.send(Msg::Busy(false, String::new()));
            let _ = tx.send(Msg::State(sys::collect()));
            if started { let _ = tx.send(Msg::RefreshModels); }
        });
    }

    fn stop(&mut self) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.busy_text = "停止中…".into();
        self.reset_model_snapshot();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let killed = sys::stop_gateway();
            let msg = if killed.is_empty() {
                "已停止（无运行中进程）".to_string()
            } else {
                format!("已停止（{}）", killed.join(", "))
            };
            let _ = tx.send(Msg::Toast(msg, true));
            let _ = tx.send(Msg::Busy(false, String::new()));
            let _ = tx.send(Msg::State(sys::collect()));
        });
    }

    fn restart(&mut self, announce_to_account_page: bool) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.busy_text = "重启中…".into();
        self.reset_model_snapshot();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            sys::stop_gateway();
            std::thread::sleep(Duration::from_millis(800));
            let (msg, ok, started) = match sys::start_gateway() {
                Ok((pid, ok)) => (format!(
                    "重启完成（PID {}）{}",
                    pid,
                    if ok { "" } else { "，探活未通过" }
                ), ok, true),
                Err(e) => (format!("重启失败: {}", e), false, false),
            };
            let _ = tx.send(Msg::Toast(msg.clone(), ok));
            // 登录页的日志区停在「正在自动重启网关使账号生效…」，toast 六秒就消失，
            // 用户看不到结束信号；由登录流程触发的重启额外落一行到日志。
            if announce_to_account_page {
                let _ = tx.send(Msg::OAuthLog(msg, ok));
            }
            let _ = tx.send(Msg::Busy(false, String::new()));
            let _ = tx.send(Msg::State(sys::collect()));
            if started { let _ = tx.send(Msg::RefreshModels); }
        });
    }

    fn save_cfg(&mut self) {
        let port: u16 = match self.port_str.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => {
                self.toast = Some(("端口不合法".into(), false, Instant::now()));
                return;
            }
        };
        let key = self.key_str.trim().to_string();
        if !key.is_empty() && key.len() < 8 {
            self.toast = Some(("密钥太短（至少 8 位）".into(), false, Instant::now()));
            return;
        }
        let mut cfg = sys::read_config();
        cfg["listen"] = Value::String(format!(":{}", port));
        cfg["api_key"] = Value::String(key);
        match sys::write_config(&cfg) {
            Ok(()) => {
                self.toast = Some((
                    "已写入 config.json，正在重启网关…".into(),
                    true,
                    Instant::now(),
                ));
                self.busy = false;
                self.restart(false);
            }
            Err(e) => self.toast = Some((format!("写入失败: {}", e), false, Instant::now())),
        }
    }

    fn check_port(&self) {
        let port: u16 = match self.port_str.trim().parse() {
            Ok(p) if p > 0 => p,
            _ => {
                let _ = self.tx.send(Msg::Toast("端口不合法".into(), false));
                return;
            }
        };
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let pids = sys::pids_on_port(port);
            let msg = if pids.is_empty() {
                format!("端口 {} 空闲", port)
            } else {
                format!(
                    "端口 {} 被占用：{}",
                    port,
                    pids.iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let _ = tx.send(Msg::Toast(msg, pids.is_empty()));
        });
    }

    fn copy_client(&mut self) {
        let port = self.st.port;
        let key = sys::api_key(&sys::read_config());
        let lines = [
            "# 客户端接入".to_string(),
            String::new(),
            format!("- 网关地址: http://127.0.0.1:{}/v1", port),
            format!("- 局域网地址: http://{}:{}/v1", self.lan, port),
            format!(
                "- API 密钥: {}",
                if key.is_empty() {
                    "(未设置，不鉴权)"
                } else {
                    &key
                }
            ),
            format!("- 探活地址: http://127.0.0.1:{}/healthz", port),
            format!("- 日志文件: {}", sys::gw_log().display()),
            String::new(),
            "同机客户端使用本机地址；其他设备使用局域网地址。".to_string(),
        ];
        let ok = copy(&lines.join("\n"));
        self.toast = Some((
            if ok {
                "客户端接入信息已复制到剪贴板".to_string()
            } else {
                "复制失败".to_string()
            },
            ok,
            Instant::now(),
        ));
    }

    fn acct_begin(&mut self) {
        if self.acct_busy {
            return;
        }
        self.acct_busy = true;
        self.auth_state = None;
        self.auth_url.clear();
        self.login_profile = None;
        self.login_browser_opened = false;
        self.oauth_log.push(("申请授权码…".into(), true));
        let realm = if self.realm == 1 { "global" } else { "cn" };
        let target_uid = self.login_target_uid.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = login_browser::Profile::prepare(&sys::root(), realm,
                (!target_uid.is_empty()).then_some(target_uid.as_str()))
                .and_then(|profile| net::auth_state(realm).map(|(state, url)| (profile, state, url)));
            match result {
                Ok((profile, state, url)) => {
                    let _ = tx.send(Msg::OAuthUrl(url, state, Some(profile)));
                }
                Err(e) => {
                    let _ = tx.send(Msg::OAuthLog(format!("获取失败: {}", e), false));
                    let _ = tx.send(Msg::OAuthUrl(String::new(), String::new(), None));
                }
            }
            let _ = tx.send(Msg::Busy(false, String::new()));
        });
    }

    fn acct_done(&mut self) {
        if self.acct_busy || !self.login_browser_opened {
            return;
        }
        let Some(profile) = self.login_profile.clone() else { return };
        let state = match self.auth_state.clone() {
            Some(s) if !s.is_empty() => s,
            _ => {
                self.oauth_log
                    .push(("请先点「获取登录链接」".into(), false));
                return;
            }
        };
        self.acct_busy = true;
        self.oauth_log.push(("用 state 换取 token…".into(), true));
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            match net::finish_login(&profile.realm, &state, &profile) {
                Ok(a) => {
                    let _ = tx.send(Msg::OAuthSaved(a));
                }
                Err(e) => {
                    // Keep this environment and state so a pending authorization can be retried.
                    let _ = tx.send(Msg::LoginExchangeFailed(e));
                }
            }
            let _ = tx.send(Msg::Busy(false, String::new()));
        });
    }

    fn reset_model_snapshot(&mut self) {
        // Ignore any completion still arriving from the process that is being stopped.
        self.model_refresh_id += 1;
        self.model_loading = false;
        self.models.clear();
        self.status = None;
        self.model_error = None;
        self.gateway_status_error = None;
    }

    fn free_gateway_status(&self, id: &str) -> Option<&Value> {
        self.status.as_ref()?.get("free_gateways")?.as_array()?.iter()
            .find(|provider| provider.get("id").and_then(Value::as_str) == Some(id))
    }

    fn free_gateway_failures(&self) -> Vec<String> {
        [ ("opencode", "OpenCode"), ("kilo", "Kilo") ].into_iter()
            .filter_map(|(id, name)| {
                let provider = self.free_gateway_status(id)?;
                let enabled = provider.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                let error = provider.get("last_error").and_then(Value::as_str).unwrap_or("");
                (enabled && !error.is_empty()).then(|| format!("{name} 上游异常"))
            }).collect()
    }

    fn model_refresh(&mut self, silent: bool) {
        let cfg = sys::read_config();
        let port = sys::cfg_port(&cfg);
        let key = sys::api_key(&cfg);
        self.model_refresh_id += 1;
        let id = self.model_refresh_id;
        self.model_loading = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let models = net::fetch_models(port, &key);
            // Fetch after catalog discovery so failures from this refresh are visible.
            let status = net::fetch_status(port, &key);
            let _ = tx.send(Msg::Models(id, models, status, silent));
        });
    }

    fn save_free_gateways(&mut self) {
        if self.busy { return; }
        match sys::save_free_gateway_settings(self.free_gateways) {
            Ok(()) => {
                self.saved_free_gateways = self.free_gateways;
                self.restart(false);
            }
            Err(error) => self.toast = Some((format!("免费网关配置保存失败: {error}"), false, Instant::now())),
        }
    }

    fn export_models_md(&mut self) {
        let mut s = String::from("# WorkBuddy2API 可用模型清单\n\n");
        s.push_str("| 模型 ID | 展示名 | 倍率 | 说明 |\n|---|---|---|---|\n");
        for m in &self.models {
            s.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                m.id,
                m.name,
                m.credits,
                m.desc.replace('|', "/")
            ));
        }
        match fs::write(sys::root().join("models-trae.md"), s) {
            Ok(()) => self.toast = Some(("已写入 models-trae.md".into(), true, Instant::now())),
            Err(e) => self.toast = Some((format!("写入失败: {}", e), false, Instant::now())),
        }
    }

    // ── 页面 ──

    fn ui_overview(&mut self, ui: &mut egui::Ui) {
        page_head(ui, "总览", "管理服务状态与客户端接入。");
        card(ui, "", |ui| {
            let running = self.st.running;
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::hover());
                let color = if running { OK } else { TX3 };
                ui.painter()
                    .circle_filled(rect.center(), 15.0, color.gamma_multiply(0.08));
                ui.painter().circle_filled(rect.center(), 4.0, color);
                let width = (ui.available_width() - 146.0).max(200.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 42.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(width);
                        ui.label(
                            RichText::new(if running {
                                "API 服务运行中"
                            } else {
                                "API 服务未启动"
                            })
                            .color(TX)
                            .size(16.0)
                            .strong(),
                        );
                        ui.label(
                            RichText::new(if running {
                                "服务已就绪，可复制下方接入信息。"
                            } else {
                                "启动服务后，即可连接你的客户端。"
                            })
                            .color(TX3)
                            .size(12.0),
                        );
                    },
                );
                let (label, fill) = if running {
                    ("停止服务", ERR)
                } else {
                    ("启动服务", ACC)
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if btn(ui, label, fill, INK, !self.busy) {
                        self.toggle();
                    }
                });
            });
            ui.add_space(8.0);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!("端口 {}", self.st.port))
                        .color(TX2)
                        .size(11.5),
                );
                pill(
                    ui,
                    if self.st.health && running {
                        "探活有响应"
                    } else if running {
                        "等待探活"
                    } else {
                        "未启动"
                    },
                    if running { OK } else { TX3 },
                );
                if self.busy {
                    ui.spinner();
                    ui.label(RichText::new(&self.busy_text).color(ACC));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if btn(ui, "刷新", CARD, TX2, !self.busy) {
                        self.refresh();
                    }
                    if btn(ui, "重启", CARD, TX2, !self.busy) {
                        self.restart(false);
                    }
                });
            });
        });
        ui.add_space(12.0);
        ui.columns(3, |cols| {
            stat_tile(
                &mut cols[0],
                "已连接账号",
                &self.st.accounts.len().to_string(),
                ACC,
                "已保存的授权凭证",
            );
            stat_tile(
                &mut cols[1],
                "可用模型",
                &self.models.len().to_string(),
                ACC2,
                "来自最近一次模型刷新",
            );
            stat_tile(
                &mut cols[2],
                "服务状态",
                if self.st.running {
                    "运行中"
                } else {
                    "未启动"
                },
                if self.st.running { OK } else { TX2 },
                "每 3 秒自动检查",
            );
        });
        ui.add_space(16.0);
        if ui.available_width() >= 760.0 {
            ui.columns(2, |cols| {
                self.ui_gateway_config(&mut cols[0]);
                self.ui_connection(&mut cols[1]);
            });
        } else {
            self.ui_gateway_config(ui);
            ui.add_space(16.0);
            self.ui_connection(ui);
        }
        ui.add_space(16.0);
        self.ui_accounts(ui);
    }

    fn ui_gateway_config(&mut self, ui: &mut egui::Ui) {
        card(ui, "网关配置", |ui| {
            ui.label(RichText::new("监听端口").color(TX2).size(12.0));
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.port_str)
                        .desired_width(76.0)
                        .background_color(BG)
                        .text_color(TX)
                        .font(FontId::monospace(13.0))
                        .margin(egui::Margin::symmetric(10.0, 8.0)),
                );
                if btn(ui, "检测占用", CARD2, TX2, true) {
                    self.check_port();
                }
            });
            ui.add_space(10.0);
            ui.label(RichText::new("API 密钥").color(TX2).size(12.0));
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 142.0).max(80.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.key_str)
                        .desired_width(width)
                        .background_color(BG)
                        .text_color(TX)
                        .font(FontId::monospace(13.0))
                        .margin(egui::Margin::symmetric(10.0, 8.0))
                        .hint_text("留空则不鉴权")
                        .password(!self.key_visible),
                );
                if btn(
                    ui,
                    if self.key_visible { "隐藏" } else { "显示" },
                    CARD2,
                    TX2,
                    true,
                ) {
                    self.key_visible = !self.key_visible;
                }
                if btn(ui, "随机", CARD2, TX2, true) {
                    self.key_str = sys::gen_key(24);
                }
            });
            ui.add_space(12.0);
            if btn(ui, "保存配置并重启", ACC, INK, !self.busy) {
                self.save_cfg();
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new("端口或密钥修改后，重启服务生效。")
                    .color(TX3)
                    .size(11.5),
            );
        });
    }

    fn ui_connection(&mut self, ui: &mut egui::Ui) {
        let mut copy_val: Option<String> = None;
        card(ui, "客户端接入", |ui| {
            let port = self.st.port;
            let key = sys::api_key(&sys::read_config());
            let base_lan = format!("http://{}:{}/v1", self.lan, port);
            let base_local = format!("http://127.0.0.1:{}/v1", port);
            let model = self
                .models
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_else(|| "尚未加载，请刷新模型".into());
            let health = format!("http://127.0.0.1:{}/healthz", port);
            // (标签, 展示值, 实际复制值)
            let rows: Vec<(&str, String, String)> = vec![
                ("局域网地址", base_lan.clone(), base_lan),
                ("本机地址", base_local.clone(), base_local),
                (
                    "API 密钥",
                    if key.is_empty() {
                        "(未设置，不鉴权)".into()
                    } else {
                        if self.key_visible {
                            key.clone()
                        } else {
                            "••••••••••••••••".into()
                        }
                    },
                    key,
                ),
                ("模型 ID", model.clone(), model),
                ("探活地址", health.clone(), health),
            ];
            for (i, (label, show, real)) in rows.iter().enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [74.0, 24.0],
                        egui::Label::new(RichText::new(*label).color(TX2).size(11.0)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if copy_action(
                            ui,
                            &format!("复制{}", label),
                            *label != "模型 ID" || !self.models.is_empty(),
                        ) {
                            copy_val = Some(real.clone());
                        }
                        ui.add_sized(
                            [ui.available_width(), 24.0],
                            egui::Label::new(
                                RichText::new(show).font(FontId::monospace(11.5)).color(TX2),
                            )
                            .truncate(),
                        )
                        .on_hover_text(show);
                    });
                });
            }
            ui.add_space(8.0);
            ui.label(
                RichText::new("同机客户端使用本机地址；其他设备使用局域网地址。")
                    .color(TX3)
                    .size(11.5),
            );
            ui.add_space(8.0);
            if btn(ui, "复制全部接入信息", CARD2, TX, true) {
                self.copy_client();
            }
        });
        if let Some(v) = copy_val {
            let ok = copy(&v);
            self.toast = Some((
                if ok {
                    "已复制到剪贴板".into()
                } else {
                    "复制失败".into()
                },
                ok,
                Instant::now(),
            ));
        }
    }

    fn ui_accounts(&mut self, ui: &mut egui::Ui) {
        card(ui, "账号池", |ui| {
            let accts = self.st.accounts.clone();
            let pending = self.pending_del.clone();
            let mut mark: Option<String> = None;
            let mut del: Option<String> = None;
            if accts.is_empty() {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("还没有连接账号").color(TX).size(14.0));
                        ui.label(
                            RichText::new("免费网关无需账号，可在「模型管理」中使用；添加本人授权账号可扩展 WorkBuddy / CodeBuddy 模型。")
                                .color(TX3)
                                .size(12.0),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if btn(ui, "+ 添加账号", ACC, INK, true) {
                            self.nav = Nav::AddAcct;
                        }
                    });
                });
            } else {
                let h = (ui.available_height() - 26.0).clamp(60.0, 170.0);
                ScrollArea::vertical().max_height(h).show(ui, |ui| {
                    egui::Grid::new("acct_grid")
                        .striped(true)
                        .num_columns(4)
                        .min_col_width(48.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new("昵称").color(TX2).size(10.5));
                            ui.label(RichText::new("UID").color(TX2).size(10.5));
                            ui.label(RichText::new("域").color(TX2).size(10.5));
                            ui.label(RichText::new("").size(10.5));
                            ui.end_row();
                            for a in &accts {
                                ui.label(RichText::new(clip(&a.nick, 12)).size(11.0));
                                ui.label(RichText::new(clip(&a.uid, 12)).size(11.0));
                                let realm_col = match a.realm.as_str() {
                                    "global" => ACC,
                                    "cn" => WARN,
                                    _ => TX2,
                                };
                                pill(ui, &a.realm, realm_col);
                                let pend = pending.as_deref() == Some(a.file.as_str());
                                let txt = if pend { "确认?" } else { "删除" };
                                if btn(ui, txt, if pend { ERR } else { CARD2 }, TX2, true) {
                                    if pend {
                                        del = Some(a.file.clone());
                                    } else {
                                        mark = Some(a.file.clone());
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
            }
            ui.add_space(4.0);
            ui.label(
                RichText::new("删除需点两次确认；新增/删除凭证后必须重启网关才生效")
                    .color(TX3)
                    .size(10.5),
            );
            if let Some(f) = mark {
                self.pending_del = Some(f);
            }
            if let Some(f) = del {
                match sys::delete_account(&f) {
                    Ok(()) => {
                        self.pending_del = None;
                        self.toast =
                            Some((format!("已删除 {}，请重启网关", f), true, Instant::now()));
                        self.refresh();
                    }
                    Err(e) => {
                        self.toast = Some((format!("删除失败: {}", e), false, Instant::now()))
                    }
                }
            }
        });
    }

    fn ui_add_acct(&mut self, ui: &mut egui::Ui) {
        let mut begin = false;
        let mut open = false;
        let mut done = false;
        let mut copy_link = false;

        page_head(
            ui,
            "添加账号",
            "使用内置指纹浏览器独立登录，完成后自动重启网关并拉取模型",
        );

        card(ui, "选择版本", |ui| {
            let previous = (self.realm, self.login_target_uid.clone());
            ui.add_enabled_ui(!self.acct_busy, |ui| {
              ui.horizontal(|ui| {
                ui.radio_value(&mut self.realm, 0, RichText::new("国内版 cn").size(12.0));
                ui.radio_value(
                    &mut self.realm,
                    1,
                    RichText::new("国际版 global").size(12.0),
                );
              });
              let realm = if self.realm == 1 { "global" } else { "cn" };
              let accounts: Vec<_> = self.st.accounts.iter().filter(|account| account.realm == realm).collect();
              if self.realm != previous.0 { self.login_target_uid.clear(); }
              ui.add_space(8.0);
              egui::ComboBox::from_id_salt("login_account_environment")
                .selected_text(if self.login_target_uid.is_empty() { "新账号 · 创建独立环境".into() }
                    else { accounts.iter().find(|account| account.uid == self.login_target_uid)
                        .map(|account| format!("重新登录 · {}", if account.nick.is_empty() { &account.uid } else { &account.nick }))
                        .unwrap_or_else(|| "重新登录已有账号".into()) })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.login_target_uid, String::new(), "新账号 · 创建独立环境");
                    for account in accounts {
                        ui.selectable_value(&mut self.login_target_uid, account.uid.clone(), format!("重新登录 · {} ({})", account.nick, clip(&account.uid, 12)));
                    }
                });
            });
            if previous != (self.realm, self.login_target_uid.clone()) {
                self.auth_state = None;
                self.auth_url.clear();
                self.login_profile = None;
                self.login_browser_opened = false;
            }
            ui.add_space(8.0);
            ui.label(RichText::new("每个账号独立保存 Cookie、缓存和指纹；重新登录时仅复用该账号环境。")
                .color(TX3).size(11.5));
        });
        ui.add_space(16.0);

        card(ui, "登录流程", |ui| {
            ui.horizontal_wrapped(|ui| {
                if btn(ui, "① 获取登录链接", ACC, Color32::WHITE, !self.acct_busy) {
                    begin = true;
                }
                if btn(ui, "② 打开独立指纹浏览器", CARD2, TX, !self.acct_busy && self.login_profile.is_some()) {
                    open = true;
                }
                if btn(
                    ui,
                    "③ 我已完成登录",
                    OK,
                    Color32::from_rgb(0x06, 0x28, 0x1C),
                    !self.acct_busy && self.auth_state.is_some() && self.login_browser_opened,
                ) {
                    done = true;
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("完整登录链接").color(TX3).size(11.5));
                if copy_action(ui, "复制完整登录链接", !self.auth_url.is_empty()) {
                    copy_link = true;
                }
            });
            egui::Frame::none().fill(BG)
                .inner_margin(egui::Margin::symmetric(10.0, 8.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if self.auth_url.is_empty() {
                        ui.label(RichText::new("获取链接后在此显示，可复制完整地址。")
                            .color(TX3).size(11.5));
                    } else {
                        ScrollArea::vertical().id_salt("full_auth_url").max_height(96.0)
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(&self.auth_url)
                                    .font(FontId::monospace(12.0)).color(TX))
                                    .selectable(true).wrap());
                            });
                    }
                });
            if let Some(profile) = &self.login_profile {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("独立环境 {} · {} · Camoufox", &profile.id[..8], profile.realm))
                    .color(OK).size(11.5));
            }
        });
        ui.add_space(16.0);

        let log = self.oauth_log.clone();
        let h = (ui.available_height() - 4.0).max(60.0);
        term_frame().show(ui, |ui| {
            ui.set_width(ui.available_width()); // .term 撑满内容区
            ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(h)
                .show(ui, |ui| {
                    if log.is_empty() {
                        ui.label(RichText::new("点「① 获取登录链接」开始").color(TX3));
                    }
                    for (line, ok) in &log {
                        ui.label(
                            RichText::new(line)
                                .font(FontId::monospace(11.0))
                                .color(if *ok { OK } else { ERR }),
                        );
                    }
                });
        });

        if begin {
            self.acct_begin();
        }
        if copy_link {
            let ok = copy(&self.auth_url);
            self.toast = Some((if ok { "已复制完整登录链接".into() }
                else { "复制失败".into() }, ok, Instant::now()));
        }
        if open && !self.auth_url.is_empty() {
            if let Some(profile) = self.login_profile.clone() {
                self.acct_busy = true;
                self.oauth_log.push(("正在启动独立指纹浏览器，首次启动需要释放内置内核…".into(), true));
                let url = self.auth_url.clone();
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let result = profile.open(&sys::root(), &url);
                    let _ = tx.send(Msg::LoginBrowserOpened(profile.id, result));
                });
            }
        }
        if done {
            self.acct_done();
        }
    }

    fn ui_usage(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        // 页头 + 右侧刷新：必须包在 ui.horizontal 内，否则 right_to_left 布局会占满整页高度
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("用量统计").color(TX).size(22.0).strong());
                ui.add_space(2.0);
                ui.label(
                    RichText::new("从网关日志汇总 Token 与请求，仅统计日志中已记录的数据。")
                        .color(TX3)
                        .size(12.0),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if btn(ui, "刷新", CARD2, TX, true) {
                    refresh = true;
                }
            });
        });
        ui.add_space(12.0);

        let u = self.usage.clone().unwrap_or_default();
        ui.columns(3, |cols| {
            stat_tile(
                &mut cols[0],
                "今日 Token",
                &fmt_num(u.today_tokens),
                ACC,
                "tokens",
            );
            stat_tile(&mut cols[1], "今日请求", &fmt_num(u.today_req), TX, "次");
            stat_tile(
                &mut cols[2],
                "成功率",
                &format!(
                    "{}%",
                    if u.total_req > 0 {
                        u.ok_req * 100 / u.total_req
                    } else {
                        0
                    }
                ),
                if u.total_req > 0 && u.ok_req == u.total_req {
                    OK
                } else {
                    WARN
                },
                &format!("{} / {}", u.ok_req, u.total_req),
            );
        });
        ui.add_space(16.0);
        ui.columns(3, |cols| {
            stat_tile(
                &mut cols[0],
                "累计 Token",
                &fmt_num(u.total_tokens),
                ACC2,
                "tokens",
            );
            stat_tile(
                &mut cols[1],
                "平均速度",
                &format!("{:.0} tok/s", u.avg_tps),
                TX,
                "tok/s",
            );
            stat_tile(
                &mut cols[2],
                "平均首字",
                &format!("{:.0} ms", u.avg_ttfb_ms),
                TX,
                "ms",
            );
        });
        ui.add_space(16.0);

        self.live.show(ui);
        ui.add_space(16.0);

        card(ui, "按模型", |ui| {
            if u.by_model.is_empty() {
                ui.label(RichText::new("暂无数据").color(TX3));
            } else {
                egui::Grid::new("usage_model")
                    .striped(true)
                    .num_columns(4)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("模型").color(TX2).size(12.0));
                        ui.label(RichText::new("Token").color(TX2).size(12.0));
                        ui.label(RichText::new("次数").color(TX2).size(12.0));
                        ui.label(RichText::new("用量占比").color(TX2).size(12.0));
                        ui.end_row();
                        for (m, t, c) in &u.by_model {
                            ui.label(RichText::new(clip(m, 26)).size(12.5));
                            ui.label(RichText::new(fmt_num(*t)).color(ACC).size(12.5));
                            ui.label(RichText::new(c.to_string()).size(12.5));
                            usage_charts::model_share(ui, *t, u.total_tokens);
                            ui.end_row();
                        }
                    });
            }
        });
        ui.add_space(16.0);

        card(ui, "按账号", |ui| {
            if u.by_uid.is_empty() {
                ui.label(RichText::new("暂无数据").color(TX3));
            } else {
                egui::Grid::new("usage_uid")
                    .striped(true)
                    .num_columns(3)
                    .min_col_width(60.0)
                    .show(ui, |ui| {
                        ui.label(RichText::new("账号 UID").color(TX2).size(12.0));
                        ui.label(RichText::new("Token").color(TX2).size(12.0));
                        ui.label(RichText::new("次数").color(TX2).size(12.0));
                        ui.end_row();
                        for (m, t, c) in &u.by_uid {
                            ui.label(RichText::new(clip(m, 24)).size(12.5));
                            ui.label(RichText::new(fmt_num(*t)).color(ACC2).size(12.5));
                            ui.label(RichText::new(c.to_string()).size(12.5));
                            ui.end_row();
                        }
                    });
            }
        });

        if refresh {
            self.usage_refresh();
        }
    }

    fn free_gateway_summary(&self, id: &str) -> (String, Color32) {
        if self.model_loading {
            return ("正在刷新模型目录与网关状态…".into(), TX3);
        }
        if !self.st.running {
            return ("网关未启动，保存并重启后加载目录".into(), TX3);
        }
        if let Some(error) = &self.gateway_status_error {
            return (format!("状态获取失败: {error}"), ERR);
        }
        let Some(provider) = self.free_gateway_status(id) else {
            return ("尚无网关状态，请刷新模型清单；旧版网关需先升级".into(), TX3);
        };
        if provider.get("enabled").and_then(Value::as_bool) == Some(false) {
            return ("已关闭".into(), TX3);
        }
        let count = provider.get("model_count").and_then(Value::as_u64).unwrap_or(0);
        let error = provider.get("last_error").and_then(Value::as_str).unwrap_or("");
        if !error.is_empty() {
            return (format!("目录 {count} 个模型 · 上游异常: {error}"), ERR);
        }
        (format!("目录 {count} 个模型 · 调用可用性以上游响应为准"), if count > 0 { OK } else { TX3 })
    }

    fn ui_free_gateways(&mut self, ui: &mut egui::Ui) {
        let mut save = false;
        card(ui, "免费网关 · 免账号", |ui| {
            ui.label(RichText::new("默认开启，无需添加账号或配置上游密钥。免费目录、额度和调用限制由提供方决定。")
                .color(TX3).size(11.5));
            ui.add_space(10.0);
            for (index, (id, name, endpoint)) in [
                ("opencode", "OpenCode", "https://opencode.ai/inference/v1"),
                ("kilo", "Kilo Gateway", "https://api.kilo.ai/api/gateway"),
            ].into_iter().enumerate() {
                let (summary, color) = self.free_gateway_summary(id);
                ui.push_id(id, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).color(TX).size(13.0).strong());
                        ui.label(RichText::new(format!("{id}:模型ID")).color(TX2).monospace().size(11.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_enabled_ui(!self.busy, |ui| {
                                toggle(ui, &mut self.free_gateways[index], &format!("启用 {name}"));
                            });
                        });
                    });
                    ui.label(RichText::new(endpoint).monospace().color(TX3).size(11.5));
                    ui.label(RichText::new(summary).color(color).size(11.5));
                    if id == "kilo" {
                        ui.label(RichText::new("Kilo 免费模型的提示词与输出可能被提供方记录，请勿提交敏感内容。")
                            .color(WARN).size(11.5));
                    }
                });
                if index == 0 {
                    ui.add_space(7.0);
                    ui.separator();
                    ui.add_space(7.0);
                }
            }
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if btn(ui, "保存并重启网关", ACC, INK, !self.busy) { save = true; }
                if self.free_gateways != self.saved_free_gateways {
                    ui.label(RichText::new("开关已修改，待保存").color(WARN).size(11.5));
                }
            });
            ui.add_space(5.0);
            ui.label(RichText::new("模型目录不代表调用成功。OpenCode 免费层可能返回 403 / FreeTierError，具体原因会在此显示。")
                .color(TX3).size(11.5));
        });
        if save { self.save_free_gateways(); }
    }

    fn ui_models(&mut self, ui: &mut egui::Ui) {
        let mut refresh = false;
        let mut export = false;
        let mut copy_id: Option<String> = None;

        page_head(ui, "模型管理", "使用免费网关，或连接授权账号扩展模型；复制完整模型 ID 接入客户端。");

        self.ui_free_gateways(ui);
        ui.add_space(16.0);

        ui.horizontal_wrapped(|ui| {
            if btn(ui, if self.model_loading { "正在刷新…" } else { "刷新模型清单" }, ACC, Color32::WHITE, !self.model_loading && !self.busy) {
                refresh = true;
            }
            if btn(ui, "导出 md", CARD2, TX, !self.models.is_empty()) {
                export = true;
            }
            ui.label(RichText::new("需网关运行中；免费网关无需账号").color(TX3).size(12.0));
        });
        ui.add_space(16.0);

        let models = self.models.clone();
        card(ui, "可用模型", |ui| {
            if models.is_empty() {
                let text = if let Some(error) = &self.model_error {
                    format!("获取模型失败: {error}")
                } else if self.model_loading {
                    "正在获取模型目录…".into()
                } else {
                    "暂无模型，启动网关后点「刷新模型清单」；免费网关无需添加账号。".into()
                };
                ui.label(RichText::new(text).color(if self.model_error.is_some() { ERR } else { TX3 }));
            } else {
                ui.label(
                    RichText::new("免费为绿色，付费倍率越高红色越深；未提供倍率时显示灰色。")
                        .color(TX3)
                        .size(11.5),
                );
                ui.add_space(10.0);
                let rate_styles: Vec<_> = models
                    .iter()
                    .map(|model| model_rates::style(&model.credits))
                    .collect();
                let h = (ui.available_height() - 26.0).clamp(60.0, 320.0);
                ScrollArea::both().max_height(h).show(ui, |ui| {
                    egui::Grid::new("model_grid")
                        .with_row_color(move |row, _| {
                            row.checked_sub(1)
                                .and_then(|index| rate_styles.get(index))
                                .map(|style| style.fill.gamma_multiply(0.45))
                        })
                        .num_columns(5)
                        .min_col_width(52.0)
                        .show(ui, |ui| {
                            ui.label(RichText::new("模型 ID").color(TX2).size(12.0));
                            ui.label(RichText::new("名称").color(TX2).size(12.0));
                            ui.label(RichText::new("倍率").color(TX2).size(12.0));
                            ui.label(RichText::new("说明").color(TX2).size(12.0));
                            ui.label(RichText::new("").size(12.0));
                            ui.end_row();
                            for m in &models {
                                ui.label(
                                    RichText::new(clip(&m.id, 26)).font(FontId::monospace(12.0)),
                                );
                                ui.label(RichText::new(clip(&m.name, 12)).size(12.0));
                                model_rates::badge(ui, &m.credits);
                                ui.label(RichText::new(clip(&m.desc, 24)).size(12.0));
                                if copy_action(ui, "复制模型 ID", true) {
                                    copy_id = Some(m.id.clone());
                                }
                                ui.end_row();
                            }
                        });
                });
            }
        });

        if refresh {
            self.model_refresh(false);
        }
        if export {
            self.export_models_md();
        }
        if let Some(id) = copy_id {
            let ok = copy(&id);
            self.toast = Some((
                if ok {
                    format!("已复制模型 ID: {}", id)
                } else {
                    "复制失败".into()
                },
                ok,
                Instant::now(),
            ));
        }
    }

    fn ui_logs(&mut self, ui: &mut egui::Ui) {
        let log = self.st.log.clone();
        page_head(ui, "运行日志", "查看网关最近输出，排查启动与调用问题。");
        ui.horizontal(|ui| {
            if btn(ui, "打开目录", CARD2, TX, true) {
                sys::open_in_explorer(&sys::logs_dir());
            }
            ui.label(RichText::new("logs\\gateway.log").color(TX2).size(12.0));
        });
        ui.add_space(16.0);
        card(ui, "最近输出", |ui| {
            let h = (ui.available_height() - 26.0).clamp(60.0, 320.0);
            term_frame().show(ui, |ui| {
                ui.set_width(ui.available_width()); // .term 撑满内容区
                ScrollArea::both()
                    .stick_to_bottom(true)
                    .max_height(h)
                    .show(ui, |ui| {
                        if log.is_empty() {
                            ui.label(RichText::new("暂无日志").color(TX3));
                        } else {
                            ui.label(
                                RichText::new(&log)
                                    .font(FontId::monospace(12.0))
                                    .color(Color32::from_rgb(0x9A, 0xA6, 0xBC)), // HTML .term 默认色
                            );
                        }
                    });
            });
        });
    }

    fn ui_about(&mut self, ui: &mut egui::Ui) {
        page_head(
            ui,
            "FiimeOPC-2API 专版",
            "仅限内部学习交流使用，请勿分发或商业化",
        );

        card(ui, "社区", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("社区地址：").color(TX2).size(12.0));
                if ui.link("opc.fiime.cn").clicked() {
                    sys::open_url("https://opc.fiime.cn");
                }
            });
        });
        ui.add_space(16.0);

        card(ui, "安全须知", |ui| {
            ui.label(
                RichText::new(
                    "本项目是非官方网关封装，使用 CodeBuddy / WorkBuddy 账号作为上游，涉及目标平台服务条款与账号风险，仅限本人授权账号、本机 / 私有环境使用",
                )
                .color(TX)
                .size(13.0),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("不得用于账号共享、转售、违规分发，或违反目标平台条款的任何用途")
                    .color(WARN)
                    .size(13.0),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "本项目是网关的 Windows 封装与可视化面板，网关核心为预编译二进制，来自上游项目：",
                )
                .color(TX)
                .size(13.0),
            );
            ui.add_space(6.0);
            if ui
                .link("https://github.com/HanawaBanana/workbuddy2api")
                .clicked()
            {
                sys::open_url("https://github.com/HanawaBanana/workbuddy2api");
            }
        });
    }

    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        page_head(ui, "设置", "让工作台按你的习惯运行。");
        ui.label(RichText::new("启动与后台").color(TX2).size(12.0).strong());
        ui.add_space(8.0);
        card(ui, "", |ui| {
            let mut gw = self.autostart_gw;
            let mut gu = self.autostart_ui;
            setting_row(
                ui,
                "登录后启动网关",
                "登录 Windows 后，自动在后台启动 API 服务。",
                &mut gw,
            );
            ui.add_space(6.0);
            setting_row(
                ui,
                "登录后启动工作台",
                "自动启动并收起到系统托盘。",
                &mut gu,
            );
            if gw != self.autostart_gw {
                self.autostart_gw = gw;
                match sys::set_startup(gw) {
                    Ok(()) => {
                        self.toast = Some((
                            if gw {
                                "网关开机自启已开启".into()
                            } else {
                                "网关开机自启已关闭".into()
                            },
                            true,
                            Instant::now(),
                        ))
                    }
                    Err(e) => {
                        self.autostart_gw = !gw;
                        self.toast = Some((format!("设置失败: {}", e), false, Instant::now()));
                    }
                }
            }
            if gu != self.autostart_ui {
                self.autostart_ui = gu;
                match sys::set_gui_startup(gu) {
                    Ok(()) => {
                        self.toast = Some((
                            if gu {
                                "界面开机自启已开启".into()
                            } else {
                                "界面开机自启已关闭".into()
                            },
                            true,
                            Instant::now(),
                        ))
                    }
                    Err(e) => {
                        self.autostart_ui = !gu;
                        self.toast = Some((format!("设置失败: {}", e), false, Instant::now()));
                    }
                }
            }
        });
        ui.add_space(22.0);
        ui.label(RichText::new("悬浮窗").color(TX2).size(12.0).strong());
        ui.add_space(8.0);
        card(ui, "", |ui| {
            let mut show = self.float_open.load(Ordering::Relaxed);
            setting_row(
                ui,
                "显示 Token 悬浮窗",
                "在独立置顶窗口中查看生成速度与累计用量。",
                &mut show,
            );
            self.float_open.store(show, Ordering::Relaxed);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("可拖动到任意位置，点击 × 收起。")
                        .color(TX3)
                        .size(11.5),
                );
            });
        });
        ui.add_space(22.0);
        card(ui, "运行数据", |ui| {
            ui.label(RichText::new("配置、账号与日志保存在当前用户目录，卸载时保留。")
                .color(TX2).size(12.0));
            ui.label(RichText::new(sys::root().display().to_string()).color(TX3).size(11.0));
            ui.add_space(8.0);
            if btn(ui, "打开数据目录", CARD2, TX, true) {
                sys::open_in_explorer(&sys::root());
            }
        });
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(15.0, 15.0), egui::Sense::hover());
            nav_icon(ui.painter(), Nav::About, rect, false);
            ui.label(
                RichText::new("关闭主窗口后，服务会继续驻留托盘。可从托盘菜单完全退出。")
                    .color(TX3)
                    .size(11.5),
            );
        });
    }
}

impl App {
    fn ui_navigation(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(NAV_W)
            .frame(
                egui::Frame::none()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(10.0, 20.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 7.0, SEL);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "W",
                        FontId::proportional(17.0),
                        ACC,
                    );
                    ui.vertical(|ui| {
                        ui.label(RichText::new("WorkBuddy").color(TX).size(14.0).strong());
                        ui.label(RichText::new("FiimeOPC · 2API").color(TX3).size(10.5));
                    });
                });
                ui.add_space(24.0);
                for (i, n) in Nav::ALL.into_iter().enumerate() {
                    if i == 5 {
                        ui.add_space(18.0);
                    }
                    let sel = self.nav == n;
                    let (row, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 36.0),
                        egui::Sense::click(),
                    );
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::SelectableLabel,
                            ui.is_enabled(),
                            sel,
                            n.label(),
                        )
                    });
                    let bg = if sel {
                        SEL
                    } else if resp.hovered() {
                        CARD2
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(row, 6.0, bg);
                    let icon_rect = egui::Rect::from_center_size(
                        egui::pos2(row.left() + 18.0, row.center().y),
                        egui::vec2(16.0, 16.0),
                    );
                    nav_icon(ui.painter(), n, icon_rect, sel);
                    ui.painter().text(
                        egui::pos2(row.left() + 36.0, row.center().y),
                        egui::Align2::LEFT_CENTER,
                        n.label(),
                        FontId::proportional(13.0),
                        if sel { ACC } else { TX2 },
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.nav = n;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(RichText::new("关闭窗口会收起至托盘").color(TX3).size(10.5));
                    ui.add_space(7.0);
                    pill(
                        ui,
                        if self.st.running {
                            "●  网关运行中"
                        } else {
                            "○  网关未启动"
                        },
                        if self.st.running { OK } else { TX3 },
                    );
                    ui.add_space(12.0);
                    ui.add_space(6.0);
                });
            });
    }

    fn ui_workspace(&mut self, ctx: &egui::Context) -> egui::Vec2 {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(28.0, 26.0)),
            )
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .id_salt(("page", self.nav as u8))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let max_width: f32 = if self.nav == Nav::Settings {
                            720.0
                        } else {
                            850.0
                        };
                        ui.set_max_width(ui.available_width().min(max_width));
                        match self.nav {
                            Nav::Overview => self.ui_overview(ui),
                            Nav::AddAcct => self.ui_add_acct(ui),
                            Nav::Usage => self.ui_usage(ui),
                            Nav::Models => self.ui_models(ui),
                            Nav::Logs => self.ui_logs(ui),
                            Nav::About => self.ui_about(ui),
                            Nav::Settings => self.ui_settings(ui),
                        }
                    })
                    .content_size
            })
            .inner
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m, ctx);
        }
        if let Some((_, _, t)) = &self.toast {
            if t.elapsed() > Duration::from_secs(6) {
                self.toast = None;
            }
        }

        // 首次：按需最小化到托盘，并建立托盘图标
        if self.start_hidden {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.start_hidden = false;
        }
        if self.tray.is_none() {
            self.native.dark_title_bar();
            ctx.send_viewport_cmd(ViewportCommand::SetTheme(egui::SystemTheme::Dark));
            self.setup_tray(ctx);
            self.usage_refresh();
            self.model_refresh(true);
        }

        // 关闭窗口 = 最小化到托盘；真正退出走托盘菜单「退出」
        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.quit.load(Ordering::SeqCst) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            }
        }

        self.ui_navigation(ctx);

        // 底部提示条（对齐 HTML .toastbar：panel 底，ok 圆点，文字 tx2）
        if let Some((msg, ok, _)) = self.toast.clone() {
            egui::TopBottomPanel::bottom("toast")
                .frame(
                    egui::Frame::none()
                        .fill(PANEL)
                        .stroke(egui::Stroke::NONE)
                        .inner_margin(egui::Margin::symmetric(13.0, 8.0)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("●")
                                .color(if ok { OK } else { WARN })
                                .size(8.0),
                        );
                        ui.add_space(7.0);
                        ui.label(RichText::new(msg).color(TX2).size(11.0));
                    });
                });
        }

        self.ui_workspace(ctx);

        // Token 悬浮窗：独立原生窗口（可拖出主窗口 / 置顶 / 无边框 / 半透明）
        let float_open = self.float_open.load(Ordering::Relaxed);
        if float_open {
            let open = self.float_open.clone();
            let stats = self.float_stats.clone();
            let tx = self.tx.clone();
            ctx.show_viewport_deferred(
                egui::ViewportId::from_hash_of("token_hud"),
                hud_viewport(),
                move |ctx, _class| {
                    if let Err(error) = native_hud::sync_window_shape() {
                        open.store(false, Ordering::Relaxed);
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                        let _ =
                            tx.send(Msg::Toast(format!("悬浮窗圆角初始化失败：{error}"), false));
                        ctx.request_repaint_of(egui::ViewportId::ROOT);
                        return;
                    }
                    let snapshot = stats.lock().map(|s| s.clone()).unwrap_or_default();
                    // OS 关闭（Alt+F4 等）时同步开关，避免被反复重建
                    if ctx.input(|i| i.viewport().close_requested()) {
                        open.store(false, Ordering::Relaxed);
                    }
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::none()
                                .fill(Color32::TRANSPARENT)
                                .inner_margin(egui::Margin::ZERO),
                        )
                        .show(ctx, |ui| token_hud(ui, &open, &snapshot));
                    ctx.request_repaint_after(Duration::from_millis(1000));
                },
            );
        }

        if let Some(tray) = &mut self.tray {
            tray.sync(TrayState {
                running: self.st.running,
                busy: self.busy,
                hud: self.float_open.load(Ordering::Relaxed),
            });
        }
        ctx.request_repaint_after(Duration::from_millis(800));
    }
}

/// --usage：不弹界面，直接把用量报告写到 usage-report.txt
/// （release 是 windows_subsystem，没有控制台，所以用文件而不是 println）
fn dump_usage_report() {
    let u = sys::parse_usage();
    let mut s = String::new();
    s.push_str(&format!(
        "WorkBuddy2API · Token 用量报告\n数据源: {}\n\n",
        sys::gw_log().display()
    ));
    s.push_str(&format!(
        "今日({}): {} tokens / {} 请求\n",
        u.today_label, u.today_tokens, u.today_req
    ));
    s.push_str(&format!(
        "累计: {} tokens / {} 请求（成功 {}）\n",
        u.total_tokens, u.total_req, u.ok_req
    ));
    s.push_str(&format!(
        "平均: {:.1} tok/s，首字 {:.0} ms\n\n按模型:\n",
        u.avg_tps, u.avg_ttfb_ms
    ));
    for (m, t, c) in &u.by_model {
        s.push_str(&format!(
            "  {:<26} {:>10} tokens  {:>4} 次\n",
            m,
            fmt_num(*t),
            c
        ));
    }
    s.push_str("\n按账号:\n");
    for (m, t, c) in &u.by_uid {
        s.push_str(&format!(
            "  {:<26} {:>10} tokens  {:>4} 次\n",
            m,
            fmt_num(*t),
            c
        ));
    }
    let out = sys::root().join("usage-report.txt");
    let _ = fs::write(&out, s);
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_icon(Arc::new(make_icon_data()))
            .with_inner_size([WIN_W, WIN_H])
            .with_min_inner_size([920.0, 580.0])
            .with_close_button(true)
            .with_minimize_button(true)
            .with_maximize_button(true)
            // Windows HUD uses a native region instead of GPU transparency.
            .with_transparent(!cfg!(target_os = "windows"))
            .with_title(native_window::TITLE),
        ..Default::default()
    }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Err(error) = sys::initialize_paths(&args) {
        return Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(error))));
    }
    if args.iter().any(|arg| arg == "--shutdown") {
        let cfg = sys::read_config();
        if net::fetch_live_usage(sys::cfg_port(&cfg), &sys::api_key(&cfg))
            .map(|usage| usage.active_requests > 0).unwrap_or(false) {
            std::process::exit(2);
        }
        if native_window::request_exit().is_err() || sys::shutdown_owned_gateway().is_err() {
            std::process::exit(3);
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--usage") {
        dump_usage_report();
        return Ok(());
    }
    let _instance = match native_window::Instance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            native_window::duplicate_notice();
            return Ok(());
        }
        Err(error) => return Err(eframe::Error::AppCreation(Box::new(error))),
    };
    let minimized = args.iter().any(|a| a == "--minimized");
    let opts = native_options();
    eframe::run_native(
        "WorkBuddy2API",
        opts,
        Box::new(move |cc| Ok(Box::new(App::new(cc, minimized)))),
    )
}
