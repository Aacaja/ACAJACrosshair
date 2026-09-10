//! ACAJA 设置窗口（v1.2 「液态玻璃 / Liquid Glass」重设计）。
//!
//! 结构：无边框透明窗口 → 自绘背板 + 自绘标题栏 → 左导航 + 右内容（玻璃卡片）→ 底部操作条。
//! 视觉：**全部自绘**（深空渐变 + 柔和光斑 + 玻璃叠加 + 镜面边 + 分层阴影 + 弹簧动效），
//!       不依赖系统透明能力——系统透明不可用时界面依然是完整正确的；Win11 上再由
//!       `windowfx::apply_once` 叠加系统圆角 / 亚克力背板（失败静默降级）。
//! 行为：工作副本模式；编辑控件立即写工作副本并置脏标记；
//!       「应用」= 保存文件 + 尽力实时推送（IPC 负载格式不变）；
//!       「退出主程序」= 写命令文件；关闭窗口由 `on_exit` 自动保存。

pub mod fonts;
pub mod preview;
pub mod strings;

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use egui::{
    Align, Align2, Color32, ComboBox, Context, CursorIcon, FontId, Frame, Id, Layout, Margin, Pos2,
    Rangef, Rect, ResizeDirection, Response, RichText, Rounding, ScrollArea, Sense, Shape, Stroke,
    TextEdit, Ui, UiBuilder, Vec2, ViewportCommand,
};
use log::{info, warn};

use crate::config::{
    AdsButton, AdsMode, GameBinding, Hotkey, PosVal, Preset, PresetStore, RightClickMode,
    Shape as CrossShape,
    MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
};
use crate::i18n::Lang;
use crate::system::monitor::MonitorInfo;
use crate::ui::strings::{ads_mode_name, shape_name, t};

const STATUS_TTL: Duration = Duration::from_millis(1900);
/// 实时预览推送的最小间隔：拖动滑杆时约 7 次/秒，足够跟手，也不会打扰主程序
const LIVE_PUSH_MIN: Duration = Duration::from_millis(140);
/// 单次实时推送的超时（毫秒）：主程序忙（例如托盘菜单模态循环）时放弃本次预览，绝不冻结界面
const LIVE_PUSH_TIMEOUT_MS: u32 = 120;
/// 内容区左右留白 / 卡片圆角 / 内容圆角
const PAD: f32 = 14.0;
const R_WINDOW: f32 = 18.0;
const R_CARD: f32 = 16.0;
const R_CTRL: f32 = 9.0;
const NAV_W: f32 = 168.0;

/// 导航 section 索引
const SEC_STYLE: usize = 0;
const SEC_DYNAMIC: usize = 1;
const SEC_POSITION: usize = 2;
const SEC_GAMEPAD: usize = 3;
const SEC_HOTKEY: usize = 4;
const SEC_IMAGE: usize = 5;
const SEC_PRESETS: usize = 6;
const SEC_SYSTEM: usize = 7;

/// 左侧导航项：(section 索引, 文案 key)。
///
/// 导航文案 key 与卡片标题解耦——卡片标题可以很长（「手柄（Apex 瞄准吸附）」），
/// 导航列只有 168px 宽，必须用短标签；这里同时也是 `strings` 单测的 key 清单。
pub(crate) const NAV_ITEMS: [(usize, &str); 8] = [
    (SEC_STYLE, "nav_style"),
    (SEC_DYNAMIC, "nav_dynamic"),
    (SEC_POSITION, "nav_position"),
    (SEC_GAMEPAD, "nav_gamepad"),
    (SEC_HOTKEY, "nav_hotkey"),
    (SEC_IMAGE, "nav_image"),
    (SEC_PRESETS, "nav_presets"),
    (SEC_SYSTEM, "nav_system"),
];

// ===========================================================================
// 调色板（深色：近黑蓝底 + 蓝紫 accent + 暖色点缀；浅色：雾白 + 同色系浅调）
// ===========================================================================

/// 液态玻璃调色板。字段全部是 `Color32`（`from_rgba_unmultiplied` 不是 const fn，
/// 所以用运行时构造函数而不是 const 常量），`Copy` 以便随处传值、不借 `self`。
#[derive(Clone, Copy)]
struct Palette {
    dark: bool,
    /// 不透明基底（系统透明不可用时的兜底）
    bg_base: Color32,
    bg_top: Color32,
    bg_bottom: Color32,
    blob_a: Color32,
    blob_b: Color32,
    blob_c: Color32,
    grain: Color32,
    glass: Color32,
    glass_hi: Color32,
    glass_lo: Color32,
    border: Color32,
    shadow: Color32,
    text: Color32,
    label: Color32,
    dim: Color32,
    accent: Color32,
    accent_bright: Color32,
    accent_soft: Color32,
    accent2: Color32,
    warm: Color32,
    control: Color32,
    hover: Color32,
    input: Color32,
    track: Color32,
    nav_fg: Color32,
    nav_fg_on: Color32,
    ok: Color32,
    warn: Color32,
    danger: Color32,
    popup: Color32,
    well: Color32,
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl Palette {
    /// 深色：底 #05060B~#0B0E18，accent #0A84FF，副 accent #5E5CE6，暖色 #FF9F0A
    fn dark() -> Self {
        Self {
            dark: true,
            bg_base: rgba(6, 8, 15, 238),
            bg_top: rgba(26, 33, 58, 180),
            bg_bottom: rgba(4, 6, 13, 200),
            blob_a: rgba(10, 132, 255, 110),
            blob_b: rgba(94, 92, 230, 100),
            blob_c: rgba(255, 159, 10, 40),
            grain: Color32::WHITE,
            glass: rgba(255, 255, 255, 12),
            glass_hi: rgba(255, 255, 255, 52),
            glass_lo: rgba(0, 0, 0, 110),
            border: rgba(255, 255, 255, 26),
            shadow: rgba(0, 0, 0, 130),
            text: Color32::from_rgb(233, 236, 245),
            label: Color32::from_rgb(182, 188, 202),
            dim: Color32::from_rgb(138, 145, 162),
            accent: Color32::from_rgb(10, 132, 255),
            accent_bright: Color32::from_rgb(77, 163, 255),
            accent_soft: rgba(10, 132, 255, 64),
            accent2: Color32::from_rgb(94, 92, 230),
            warm: Color32::from_rgb(255, 159, 10),
            control: rgba(255, 255, 255, 26),
            hover: rgba(255, 255, 255, 44),
            input: rgba(0, 0, 0, 120),
            track: rgba(255, 255, 255, 34),
            nav_fg: Color32::from_rgb(154, 163, 182),
            nav_fg_on: Color32::from_rgb(242, 245, 251),
            ok: Color32::from_rgb(48, 209, 88),
            warn: Color32::from_rgb(255, 159, 10),
            danger: Color32::from_rgb(255, 69, 58),
            popup: rgba(22, 25, 36, 246),
            well: rgba(0, 0, 0, 90),
        }
    }

    /// 浅色：雾白 #F7F8FC~#E8EAF2，accent #0A6FD8，同名色系浅调
    fn light() -> Self {
        Self {
            dark: false,
            bg_base: rgba(246, 247, 252, 250),
            bg_top: rgba(255, 255, 255, 200),
            bg_bottom: rgba(226, 231, 243, 210),
            blob_a: rgba(10, 132, 255, 70),
            blob_b: rgba(94, 92, 230, 60),
            blob_c: rgba(255, 159, 10, 44),
            grain: Color32::from_rgb(20, 32, 64),
            glass: rgba(255, 255, 255, 200),
            glass_hi: rgba(255, 255, 255, 240),
            glass_lo: rgba(28, 42, 74, 26),
            border: rgba(28, 42, 74, 46),
            shadow: rgba(24, 36, 72, 70),
            text: Color32::from_rgb(24, 28, 38),
            label: Color32::from_rgb(58, 66, 84),
            dim: Color32::from_rgb(112, 120, 140),
            accent: Color32::from_rgb(10, 111, 216),
            accent_bright: Color32::from_rgb(10, 132, 255),
            accent_soft: rgba(10, 132, 255, 44),
            accent2: Color32::from_rgb(94, 92, 230),
            warm: Color32::from_rgb(230, 130, 0),
            control: rgba(28, 42, 74, 26),
            hover: rgba(28, 42, 74, 44),
            input: rgba(255, 255, 255, 200),
            track: rgba(28, 42, 74, 40),
            nav_fg: Color32::from_rgb(92, 100, 120),
            nav_fg_on: Color32::from_rgb(16, 20, 30),
            ok: Color32::from_rgb(29, 163, 74),
            warn: Color32::from_rgb(199, 119, 0),
            danger: Color32::from_rgb(215, 55, 46),
            popup: rgba(252, 253, 255, 250),
            well: rgba(28, 42, 74, 20),
        }
    }
}

// ---- 颜色工具 ----

/// 按比例缩放 alpha（k 会被夹到 Rangef::new(0, 1)）
fn fade(c: Color32, k: f32) -> Color32 {
    rgba(c.r(), c.g(), c.b(), (c.a() as f32 * k.clamp(0.0, 1.0)).round() as u8)
}

fn with_alpha(c: Color32, a: u8) -> Color32 {
    rgba(c.r(), c.g(), c.b(), a)
}

/// 线性插值（含 alpha），用于悬停过渡与渐变分带
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    rgba(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

// ===========================================================================
// 玻璃绘制原语（全部产出 Shape，便于放到「内容之下」或一次性提交）
// ===========================================================================

/// 柔和外阴影：多层偏移圆角描边（egui 没有高斯模糊，用同心层模拟）
fn push_shadow(out: &mut Vec<Shape>, rect: Rect, radius: f32, pal: &Palette, k: f32) {
    let steps = 6;
    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let grow = 1.0 + i as f32 * 1.7;
        let a = (1.0 - t) * (1.0 - t) * k * if pal.dark { 0.85 } else { 0.60 };
        if a <= 0.01 {
            continue;
        }
        out.push(Shape::rect_stroke(
            rect.expand(grow).translate(Vec2::new(0.0, 2.0)),
            Rounding::same(radius + grow),
            Stroke::new(1.7_f32, fade(pal.shadow, a)),
        ));
    }
}

/// 圆角垂直渐变：分带填充，首带取上半圆角、末带取下半圆角，中间带不外扩 → 不会溢出圆角
fn push_vgrad(out: &mut Vec<Shape>, rect: Rect, radius: f32, top: Color32, bottom: Color32, bands: usize) {
    let bands = bands.max(2);
    for i in 0..bands {
        let t0 = i as f32 / bands as f32;
        let y0 = rect.top() + rect.height() * t0;
        let y1 = if i + 1 == bands {
            rect.bottom()
        } else {
            rect.top() + rect.height() * ((i + 1) as f32 / bands as f32) + 0.75
        };
        let c = mix(top, bottom, t0);
        let r = if i == 0 {
            Rounding { nw: radius, ne: radius, sw: 0.0, se: 0.0 }
        } else if i + 1 == bands {
            Rounding { nw: 0.0, ne: 0.0, sw: radius, se: radius }
        } else {
            Rounding::same(0.0)
        };
        out.push(Shape::rect_filled(
            Rect::from_min_max(Pos2::new(rect.left(), y0), Pos2::new(rect.right(), y1)),
            r,
            c,
        ));
    }
}

/// 玻璃面板：阴影 → 玻璃底（上下微渐变）→ 顶部光泽带 → 镜面边（顶亮底暗）
fn push_panel(out: &mut Vec<Shape>, rect: Rect, radius: f32, pal: &Palette, hot: f32) {
    push_shadow(out, rect, radius, pal, 0.95);
    let base = pal.glass;
    push_vgrad(out, rect, radius, mix(base, pal.glass_hi, 0.10 + 0.20 * hot), base, 6);
    // 顶部光泽带（固定高度，底边是直线、内部不溢出）
    let gloss = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + 44.0));
    out.push(Shape::rect_filled(
        gloss,
        Rounding { nw: radius, ne: radius, sw: 0.0, se: 0.0 },
        fade(pal.glass_hi, if pal.dark { 0.06 + 0.06 * hot } else { 0.16 + 0.10 * hot }),
    ));
    out.push(Shape::rect_stroke(rect, Rounding::same(radius), Stroke::new(1.0_f32, pal.border)));
    let inner = Rangef::new(rect.left() + radius * 0.7, rect.right() - radius * 0.7);
    out.push(Shape::hline(inner, rect.top() + 1.0, Stroke::new(1.0_f32, fade(pal.glass_hi, 0.62))));
    out.push(Shape::hline(inner, rect.bottom() - 1.0, Stroke::new(1.0_f32, fade(pal.glass_lo, 0.85))));
}

/// 内凹玻璃（输入框 / 下滑槽 / 缩略图井）
fn push_well(out: &mut Vec<Shape>, rect: Rect, radius: f32, pal: &Palette) {
    out.push(Shape::rect_filled(rect, Rounding::same(radius), pal.input));
    // 下半段再压一层暗色 → 内凹的纵深（浅色主题下是玻璃的厚度）
    let lower = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.55), rect.max);
    out.push(Shape::rect_filled(
        lower,
        Rounding { nw: 0.0, ne: 0.0, sw: radius, se: radius },
        pal.well,
    ));
    out.push(Shape::hline(
        Rangef::new(rect.left() + radius * 0.6, rect.right() - radius * 0.6),
        rect.top() + 1.0,
        Stroke::new(1.0_f32, fade(pal.glass_lo, 0.9)),
    ));
    out.push(Shape::rect_stroke(rect, Rounding::same(radius), Stroke::new(1.0_f32, fade(pal.border, 0.8))));
}

/// 背板：不透明基底 → 深空渐变 → 光斑 → 网格 + 噪点 → 玻璃窗镜面边
fn paint_backdrop(p: &egui::Painter, rect: Rect, pal: &Palette) {
    p.rect_filled(rect, Rounding::same(R_WINDOW), pal.bg_base);
    let mut shapes = Vec::new();
    push_vgrad(&mut shapes, rect, R_WINDOW, pal.bg_top, pal.bg_bottom, 26);
    p.add(Shape::Vec(shapes));
    // 大面积柔和彩色光斑（多层同心低透明度圆）
    let (w, h) = (rect.width(), rect.height());
    soft_blob(p, Pos2::new(rect.left() + w * 0.16, rect.top() + h * 0.05), w * 0.44, pal.blob_a, 12);
    soft_blob(p, Pos2::new(rect.left() + w * 0.94, rect.top() + h * 0.30), w * 0.40, pal.blob_b, 12);
    soft_blob(p, Pos2::new(rect.left() + w * 0.60, rect.bottom() + h * 0.08), w * 0.36, pal.blob_c, 10);
    // 细网格 + 确定性噪点（同一种子 → 静态颗粒，不闪烁）
    let grid = fade(pal.grain, if pal.dark { 0.030 } else { 0.045 });
    let step = 34.0;
    let mut x = rect.left() + step;
    while x < rect.right() - 4.0 {
        p.line_segment([Pos2::new(x, rect.top() + 2.0), Pos2::new(x, rect.bottom() - 2.0)], Stroke::new(1.0_f32, grid));
        x += step;
    }
    let mut y = rect.top() + step;
    while y < rect.bottom() - 4.0 {
        p.line_segment([Pos2::new(rect.left() + 2.0, y), Pos2::new(rect.right() - 2.0, y)], Stroke::new(1.0_f32, grid));
        y += step;
    }
    let grain = fade(pal.grain, if pal.dark { 0.035 } else { 0.05 });
    let mut seed = 0x9E37_79B9u32;
    for _ in 0..200 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let fx = (seed >> 9) as f32 / 8_388_608.0;
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let fy = (seed >> 9) as f32 / 8_388_608.0;
        p.rect_filled(
            Rect::from_min_size(Pos2::new(rect.left() + fx * w, rect.top() + fy * h), Vec2::splat(1.0)),
            0.0,
            grain,
        );
    }
    // 玻璃窗镜面轮廓
    p.rect_stroke(rect.shrink(0.5), Rounding::same(R_WINDOW), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.45)));
}

/// 柔光斑：同心圆叠加出「模糊」观感（每层都很淡，越靠内叠加越多）
fn soft_blob(p: &egui::Painter, center: Pos2, radius: f32, color: Color32, rings: usize) {
    for i in 0..rings {
        let t = i as f32 / rings as f32;
        let r = radius * (1.0 - 0.62 * t);
        p.circle_filled(center, r, fade(color, 0.05 + 0.035 * t));
    }
}

// ===========================================================================
// 控件（全部自绘，返回 Response 以支持悬停动效与排布）
// ===========================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Btn {
    /// 主按钮：accent 渐变 + 顶部内高光
    Primary,
    /// 次按钮：玻璃质感
    Glass,
    /// 幽灵按钮：无底，悬停才浮出
    Ghost,
    /// 危险按钮
    Danger,
}

/// 把调色板写进当前 Ui 的样式（ComboBox / TextEdit / DragValue / 滚动条都跟着走）
fn apply_style(ui: &mut Ui, pal: &Palette) {
    let v = ui.visuals_mut();
    v.dark_mode = pal.dark;
    v.override_text_color = Some(pal.text);
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = pal.popup;
    v.window_stroke = Stroke::new(1.0_f32, pal.border);
    v.window_rounding = Rounding::same(12.0);
    v.extreme_bg_color = pal.input;
    v.faint_bg_color = pal.control;
    v.selection.bg_fill = pal.accent_soft;
    v.selection.stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.noninteractive.bg_fill = pal.control;
    v.widgets.noninteractive.weak_bg_fill = pal.control;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, pal.dim);
    v.widgets.noninteractive.rounding = Rounding::same(R_CTRL);
    v.widgets.inactive.bg_fill = pal.control;
    v.widgets.inactive.weak_bg_fill = pal.control;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, pal.border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.inactive.rounding = Rounding::same(R_CTRL);
    v.widgets.hovered.bg_fill = pal.hover;
    v.widgets.hovered.weak_bg_fill = pal.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, pal.accent_bright);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.hovered.rounding = Rounding::same(R_CTRL);
    v.widgets.active.bg_fill = pal.accent_soft;
    v.widgets.active.weak_bg_fill = pal.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.active.rounding = Rounding::same(R_CTRL);
    v.widgets.open.bg_fill = pal.control;
    v.widgets.open.weak_bg_fill = pal.control;
    v.widgets.open.rounding = Rounding::same(R_CTRL);
}

/// 玻璃按钮。`min_w = 0` → 按文字自动宽度。
fn button_sized(
    ui: &mut Ui,
    pal: &Palette,
    seed: &str,
    label: &str,
    kind: Btn,
    min_w: f32,
    h: f32,
    font_size: f32,
) -> Response {
    let font = FontId::proportional(font_size);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), pal.text);
    let w = (galley.size().x + 22.0).max(min_w);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let id = Id::new(("btn", seed, label));
    let hot = ui.ctx().animate_bool_with_time(id, resp.hovered(), 0.12);
    let down = resp.is_pointer_button_down_on();
    let rect = if down { rect.shrink2(Vec2::new(0.0, 0.6)) } else { rect };
    let radius = Rounding::same(R_CTRL);
    let mut shapes = Vec::new();
    match kind {
        Btn::Primary => {
            push_shadow(&mut shapes, rect, R_CTRL, pal, 0.75);
            push_vgrad(
                &mut shapes,
                rect,
                R_CTRL,
                mix(pal.accent_bright, Color32::WHITE, 0.22 + 0.16 * hot),
                mix(pal.accent, Color32::BLACK, 0.10),
                4,
            );
            shapes.push(Shape::hline(
                Rangef::new(rect.left() + R_CTRL, rect.right() - R_CTRL),
                rect.top() + 1.3,
                Stroke::new(1.0_f32, rgba(255, 255, 255, 96)),
            ));
            shapes.push(Shape::rect_stroke(rect, radius, Stroke::new(1.0_f32, fade(pal.accent_bright, 0.85))));
            ui.painter().add(Shape::Vec(shapes));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, Color32::WHITE);
        }
        Btn::Danger => {
            push_shadow(&mut shapes, rect, R_CTRL, pal, 0.55);
            push_vgrad(&mut shapes, rect, R_CTRL, mix(pal.danger, Color32::WHITE, 0.18), mix(pal.danger, Color32::BLACK, 0.10), 4);
            shapes.push(Shape::rect_stroke(rect, radius, Stroke::new(1.0_f32, fade(pal.danger, 0.9))));
            ui.painter().add(Shape::Vec(shapes));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, Color32::WHITE);
        }
        Btn::Glass => {
            push_shadow(&mut shapes, rect, R_CTRL, pal, 0.45);
            push_vgrad(
                &mut shapes,
                rect,
                R_CTRL,
                mix(pal.control, pal.glass_hi, 0.05 + 0.12 * hot),
                mix(pal.control, pal.glass_lo, 0.25),
                3,
            );
            shapes.push(Shape::rect_stroke(
                rect,
                radius,
                Stroke::new(1.0_f32, mix(pal.border, pal.accent_bright, hot * 0.8)),
            ));
            shapes.push(Shape::hline(
                Rangef::new(rect.left() + R_CTRL, rect.right() - R_CTRL),
                rect.top() + 1.0,
                Stroke::new(1.0_f32, fade(pal.glass_hi, 0.35 + 0.25 * hot)),
            ));
            ui.painter().add(Shape::Vec(shapes));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, mix(pal.label, pal.text, hot));
        }
        Btn::Ghost => {
            if hot > 0.01 || down {
                shapes.push(Shape::rect_filled(rect, radius, fade(pal.control, hot * 0.9 + 0.2)));
                ui.painter().add(Shape::Vec(shapes));
            }
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, mix(pal.dim, pal.text, hot));
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// 勾选框：accent 填充 + 几何对勾
fn checkbox(ui: &mut Ui, pal: &Palette, seed: &str, on: &mut bool, label: &str) -> bool {
    let font = FontId::proportional(12.5);
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), pal.label).size().x;
    let w = (26.0 + text_w).min(ui.available_width().max(26.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
    let t = ui.ctx().animate_value_with_time(Id::new(("chk", seed)), if *on { 1.0 } else { 0.0 }, 0.14);
    let hot = ui.ctx().animate_bool_with_time(Id::new(("chkh", seed)), resp.hovered(), 0.12);
    let box_rect = Rect::from_center_size(Pos2::new(rect.left() + 8.5, rect.center().y), Vec2::splat(17.0));
    let p = ui.painter();
    p.rect_filled(box_rect, Rounding::same(5.0), pal.input);
    if t > 0.01 {
        p.rect_filled(box_rect, Rounding::same(5.0), fade(pal.accent, t));
        let c = box_rect.center();
        p.add(Shape::line(
            vec![Pos2::new(c.x - 3.7, c.y + 0.2), Pos2::new(c.x - 1.0, c.y + 3.0), Pos2::new(c.x + 4.0, c.y - 3.1)],
            Stroke::new(1.9_f32, fade(Color32::WHITE, t)),
        ));
    }
    p.rect_stroke(
        box_rect,
        Rounding::same(5.0),
        Stroke::new(1.0_f32, mix(mix(pal.border, pal.accent, t), pal.accent_bright, hot * 0.7)),
    );
    p.text(
        Pos2::new(box_rect.right() + 8.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        mix(pal.label, pal.text, if *on { 1.0 } else { hot }),
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// 细轨道 + 高光把手滑杆（f32）
fn slider_f32(
    ui: &mut Ui,
    pal: &Palette,
    seed: &str,
    v: &mut f32,
    range: Rangef,
    width: f32,
    decimals: usize,
    suffix: &str,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(90.0), 22.0), Sense::click_and_drag());
    let val_w = 46.0_f32.min(rect.width() * 0.4);
    let track = Rect::from_min_max(
        Pos2::new(rect.left(), rect.center().y - 1.5),
        Pos2::new(rect.right() - val_w, rect.center().y + 1.5),
    );
    let span = (range.max - range.min).max(1e-6);
    let mut changed = false;
    if resp.is_pointer_button_down_on() || resp.dragged() {
        if let Some(pos) = ui.ctx().pointer_interact_pos() {
            let t = ((pos.x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
            let nv = range.min + t * span;
            if (nv - *v).abs() > f32::EPSILON {
                *v = nv;
                changed = true;
            }
        }
    }
    let t = ((*v - range.min) / span).clamp(0.0, 1.0);
    let hot = ui.ctx().animate_bool_with_time(Id::new(("sl", seed)), resp.hovered() || resp.dragged(), 0.12);
    let p = ui.painter();
    p.rect_filled(track, Rounding::same(2.0), pal.track);
    let fill = Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * t, track.max.y));
    if fill.width() > 0.5 {
        p.rect_filled(fill, Rounding::same(2.0), mix(pal.accent, pal.accent_bright, 0.35));
    }
    let c = Pos2::new(track.left() + track.width() * t, rect.center().y);
    let r = 6.0 + 1.6 * hot;
    p.circle_filled(c, r + 4.0, fade(pal.accent, 0.16 + 0.24 * hot));
    p.circle_filled(c, r, Color32::WHITE);
    p.circle_stroke(c, r, Stroke::new(1.0_f32, fade(pal.accent, 0.85)));
    p.text(
        Pos2::new(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        format!("{:.*}{}", decimals, *v, suffix),
        FontId::proportional(11.0),
        mix(pal.dim, pal.text, hot),
    );
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    changed
}

/// 同 `slider_f32`，整数版
fn slider_i32(ui: &mut Ui, pal: &Palette, seed: &str, v: &mut i32, range: Rangef, width: f32) -> bool {
    let mut f = *v as f32;
    let changed = slider_f32(ui, pal, seed, &mut f, range, width, 0, "");
    let nv = f.round() as i32;
    if changed && nv != *v {
        *v = nv;
        return true;
    }
    false
}

/// 玻璃下拉框（用 egui 的 ComboBox，样式由 `apply_style` 统一成玻璃质感）
fn combo<R>(
    ui: &mut Ui,
    width: f32,
    salt: &str,
    text: String,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let outer = ui.scope(|ui| {
        ComboBox::from_id_salt(salt)
            .width(width)
            .selected_text(RichText::new(text).size(12.0))
            .show_ui(ui, add)
    });
    outer.inner.inner
}

/// 内凹玻璃文本框
fn text_field(ui: &mut Ui, pal: &Palette, buf: &mut String, width: f32, hint: &str) -> Response {
    ui.scope(|ui| {
        let mut e = TextEdit::singleline(buf).desired_width(width.max(60.0));
        if !hint.is_empty() {
            e = e.hint_text(RichText::new(hint).size(11.5).color(pal.dim));
        }
        ui.add(e)
    })
    .inner
}

/// 字段标签列宽：中英双语都塞得下最长的一个（"Threshold (0-255)" ≈ 112px）
const LABEL_W: f32 = 122.0;

/// 固定宽度的字段标签（对齐用）；超出列宽的部分裁掉，绝不压到右侧滑杆上
fn field_label(ui: &mut Ui, pal: &Palette, text: &str, w: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::hover());
    let clip = rect.intersect(ui.clip_rect());
    let p = ui.painter().with_clip_rect(clip);
    p.text(rect.left_center(), Align2::LEFT_CENTER, text, FontId::proportional(12.5), pal.label);
}

/// 说明文字
fn note(ui: &mut Ui, pal: &Palette, text: &str) {
    ui.label(RichText::new(text).size(10.5).color(pal.dim));
}

/// 按字符数截断（预设名最长 40 字符，列表行放不下）
fn ellipsize(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// 玻璃卡片：背景用 `Shape::Noop` 占位 → 内容画完后再置换，
/// 这样阴影 / 玻璃底 / 镜面边都能落在内容**之下**（egui 即时模式的标准做法）。
fn card(ui: &mut Ui, pal: &Palette, title: Option<&str>, body: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let slot = ui.painter().add(Shape::Noop);
    let resp = Frame::none()
        .inner_margin(Margin::same(16.0))
        .show(ui, |ui| {
            ui.set_width((width - 32.0).max(120.0));
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            if let Some(title) = title {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 16.0), Sense::hover());
                    let p = ui.painter();
                    p.circle_filled(Pos2::new(r.left() + 5.0, r.center().y), 2.6, pal.accent);
                    p.circle_filled(Pos2::new(r.left() + 5.0, r.center().y), 6.0, fade(pal.accent, 0.18));
                    ui.label(
                        RichText::new(title)
                            .size(13.5)
                            .strong()
                            .color(mix(pal.text, pal.accent, 0.25)),
                    );
                });
                ui.add_space(2.0);
            }
            body(ui);
        });
    let rect = resp.response.rect;
    let mut shapes = Vec::new();
    push_panel(&mut shapes, rect, R_CARD, pal, 0.0);
    ui.painter().set(slot, Shape::Vec(shapes));
    ui.add_space(12.0);
}

// ===========================================================================
// 应用状态
// ===========================================================================

/// 图片缩略图（按路径缓存，避免每帧解码）
enum Thumb {
    Empty,
    Failed,
    Ready(egui::TextureHandle),
}

pub struct AcajaApp {
    store: Arc<Mutex<PresetStore>>,
    /// 后端连接状态（找到主进程窗口 / 最近一次推送成功）
    backend_ok: bool,
    active_section: usize,

    /// 工作副本（界面直接编辑此预设）
    preset: Preset,
    active_name: String,
    dirty: bool,
    /// 上一次已推给主程序的负载（内容未变则不重复推送）
    last_push_body: String,
    /// 上一次实时推送的时刻（节流）
    last_live_push: Instant,
    visible: bool,

    lang: Lang,
    theme: String,
    pal: Palette,

    new_preset_name: String,
    status: Option<(String, Instant)>,

    // hex / 文本编辑缓冲（与预设字段分离，避免输入中间态污染）
    hex_main: String,
    hex_top: String,
    hex_bottom: String,
    hex_left: String,
    hex_right: String,
    hex_outline: String,
    hotkey_buf: String,
    hotkey_next_buf: String,
    new_binding_exe: String,
    new_binding_preset: String,

    // ---- 迭代 1 新增 ----
    /// 正在录制热键的槽位：0 = 切换准星，1 = 切换下一预设
    recording: Option<usize>,
    /// 预设重命名目标（内联编辑态）
    rename_from: Option<String>,
    rename_buf: String,
    /// 重命名二次确认
    rename_confirm: bool,
    /// 预设删除二次确认
    delete_confirm: Option<String>,
    /// 显示器列表（进入位置分区时按 3s 缓存刷新）
    monitors: Vec<MonitorInfo>,
    monitors_at: Option<Instant>,
    /// 缩略图缓存：当前对应的路径 + 内容
    thumb_for: String,
    thumb: Thumb,
    /// 8 个模板的缩略图预设（启动时构建一次）
    tpl_previews: Vec<Preset>,
}

/// 启动设置窗口（独立进程模式：阻塞直到窗口关闭，关闭即进程结束）
pub fn run(store: Arc<Mutex<PresetStore>>, title: &'static str) -> eframe::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([900.0, 800.0])
        .with_min_inner_size([820.0, 620.0])
        // 无边框 + 透明：自带标题栏与背板，系统装饰全部交给我们自己画
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(true)
        .with_title(title);
    if let Ok(im) = image::load_from_memory(include_bytes!("../../assets/icons/ACAJA_64.png")) {
        let rgba = im.to_rgba8();
        let (w, h) = rgba.dimensions();
        viewport = viewport.with_icon(Arc::new(egui::IconData {
            rgba: rgba.into_raw(),
            width: w,
            height: h,
        }));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(move |cc| Ok(Box::new(AcajaApp::new(cc, store)) as Box<dyn eframe::App>)),
    )
}

impl AcajaApp {
    fn new(cc: &eframe::CreationContext<'_>, store: Arc<Mutex<PresetStore>>) -> Self {
        // ---- 中文字体 ----
        if let Some(font_bytes) = fonts::load_cjk_font() {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("msyh".to_owned(), Arc::new(egui::FontData::from_owned(font_bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.insert(0, "msyh".to_owned());
                }
            }
            cc.egui_ctx.set_fonts(fonts);
        } else {
            info!("未找到中文字体，界面将回退系统字体");
        }

        let (preset, active_name, lang, theme) = {
            let g = store.lock();
            (g.get_active().clone(), g.active_name(), g.app.lang(), g.app.theme.clone())
        };
        let dark = match theme.as_str() {
            "light" => false,
            _ => true,
        };
        let pal = if dark { Palette::dark() } else { Palette::light() };

        // ---- 排版节奏：标题 20 / 强调 13.5 / 正文 12.5 / 说明 10.5 ----
        let mut style = (*cc.egui_ctx.style()).clone();
        style.text_styles = [
            (egui::TextStyle::Heading, FontId::proportional(20.0)),
            (egui::TextStyle::Body, FontId::proportional(12.5)),
            (egui::TextStyle::Button, FontId::proportional(12.5)),
            (egui::TextStyle::Small, FontId::proportional(10.5)),
            (egui::TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        style.animation_time = 0.15;
        cc.egui_ctx.set_style(style);

        let tpl_previews = TEMPLATES
            .iter()
            .map(|(_, apply)| {
                let mut p = template_base(&preset);
                apply(&mut p);
                p
            })
            .collect();

        let mut app = AcajaApp {
            store,
            backend_ok: crate::ipc::find_backend().is_some(),
            active_section: SEC_STYLE,
            preset,
            active_name,
            dirty: false,
            last_push_body: String::new(),
            last_live_push: Instant::now(),
            visible: true,
            lang,
            theme,
            pal,
            new_preset_name: String::new(),
            status: None,
            hex_main: String::new(),
            hex_top: String::new(),
            hex_bottom: String::new(),
            hex_left: String::new(),
            hex_right: String::new(),
            hex_outline: String::new(),
            hotkey_buf: String::new(),
            hotkey_next_buf: String::new(),
            new_binding_exe: String::new(),
            new_binding_preset: String::new(),
            recording: None,
            rename_from: None,
            rename_buf: String::new(),
            rename_confirm: false,
            delete_confirm: None,
            monitors: Vec::new(),
            monitors_at: None,
            thumb_for: String::new(),
            thumb: Thumb::Empty,
            tpl_previews,
        };
        app.sync_buffers();
        app
    }

    // ---- 辅助 ----

    fn sync_buffers(&mut self) {
        let f = |s: &str| if s.starts_with('#') { s.to_string() } else { format!("#{s}") };
        self.hex_main = f(&self.preset.color);
        self.hex_top = f(&self.preset.colors.top);
        self.hex_bottom = f(&self.preset.colors.bottom);
        self.hex_left = f(&self.preset.colors.left);
        self.hex_right = f(&self.preset.colors.right);
        self.hex_outline = f(&self.preset.outline.color);
        self.hotkey_buf = self.preset.hotkey_toggle.to_string();
        self.hotkey_next_buf = self.preset.hotkey_next_profile.to_string();
    }

    fn flash(&mut self, text: String) {
        self.status = Some((text, Instant::now()));
    }

    fn save_current(&mut self) {
        let result = {
            let mut store = self.store.lock();
            let res = store.save_preset(&self.active_name.clone(), &self.preset);
            if res.is_ok() {
                let _ = store.save_app();
            }
            res
        };
        match result {
            Ok(()) => {
                self.dirty = false;
                self.flash(format!("{} {}", t(self.lang, "saved"), self.active_name));
            }
            Err(e) => self.flash(format!("{}: {e}", t(self.lang, "error"))),
        }
    }

    fn apply_preset(&mut self, name: &str) {
        let name = name.to_string();
        {
            let mut store = self.store.lock();
            if let Some(p) = store.get(&name).cloned() {
                self.preset = p;
                self.active_name = name.clone();
                store.activate(&name);
                let _ = store.save_app();
            }
        }
        self.sync_buffers();
        self.recording = None;
        self.rename_from = None;
        self.rename_confirm = false;
        self.delete_confirm = None;
        self.dirty = true;
    }

    /// 实时预览推送：参数一变就把当前状态推给主程序（节流 ~7 次/秒）。
    ///
    /// 拖动滑杆/换形状时**屏幕上的真实准星即时跟随**（Crosshair X 同款手感）。
    /// 不落盘 —— 写配置文件由「应用设置到主程序」按钮或关窗时的 `on_exit` 负责。
    /// 用带超时的发送：主程序忙时放弃本次预览，界面永远不会被拖住（v1.1.5 曾因阻塞发送改成手动推送，
    /// 这里用 `SendMessageTimeout` 找回实时手感又不牺牲稳定性）。
    fn live_push(&mut self) {
        let body = crate::ipc::preset_payload(&self.preset, self.visible);
        if body == self.last_push_body || self.last_live_push.elapsed() < LIVE_PUSH_MIN {
            return;
        }
        let Some(hwnd) = crate::ipc::find_backend() else {
            self.backend_ok = false;
            return;
        };
        let ok = crate::ipc::send_json_timeout(
            hwnd,
            crate::ipc::IPC_TAG_PRESET,
            &body,
            LIVE_PUSH_TIMEOUT_MS,
        );
        self.backend_ok = ok;
        if ok {
            self.last_push_body = body;
            self.last_live_push = Instant::now();
        }
    }

    /// 推送按钮：保存文件 + 尽力实时推送给后台壳（IPC 负载格式不变）
    fn push_to_backend(&mut self) {
        let (name, preset) = (self.active_name.clone(), self.preset.clone());
        let saved = {
            let mut st = self.store.lock();
            let r = st.save_preset(&name, &preset);
            if r.is_ok() {
                let _ = st.save_app();
            }
            r.is_ok()
        };
        let body = crate::ipc::preset_payload(&self.preset, self.visible);
        let pushed = if let Some(hwnd) = crate::ipc::find_backend() {
            crate::ipc::send_json(hwnd, crate::ipc::IPC_TAG_PRESET, &body)
        } else {
            false
        };
        self.backend_ok = pushed;
        if pushed {
            // 记下已推送内容，避免帧末的实时推送再重复发一次
            self.last_push_body = body;
            self.last_live_push = Instant::now();
        }
        if saved {
            self.dirty = false;
            self.flash(if pushed {
                t(self.lang, "pushed_ok").to_string()
            } else {
                t(self.lang, "pushed_via_file").to_string()
            });
        } else {
            self.flash(t(self.lang, "error").to_string());
        }
    }

    // ================================================================
    // 标题栏 / 导航 / 底栏
    // ================================================================

    /// 自绘标题栏：品牌标记 + 标题 + 版本 + 语言/主题 + 最小化/最大化/关闭；整条可拖动
    fn title_bar(&mut self, ui: &mut Ui, pal: &Palette, rect: Rect) {
        let lang = self.lang;
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        let ctrl_w = 322.0;
        // 拖动区（右侧留给控件）
        let drag_rect = Rect::from_min_max(rect.min, Pos2::new((rect.right() - ctrl_w).max(rect.left() + 40.0), rect.bottom()));
        let drag = ui.interact(drag_rect, Id::new("tbar_drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        if drag.hovered() {
            ui.ctx().set_cursor_icon(if drag.dragged() { CursorIcon::Grabbing } else { CursorIcon::Grab });
        }

        let p = ui.painter();
        let cy = rect.center().y;
        // 品牌标记：accent 渐变圆角方块 + 几何准星
        let mark = Rect::from_center_size(Pos2::new(rect.left() + 17.0, cy), Vec2::splat(30.0));
        let mut shapes = Vec::new();
        push_shadow(&mut shapes, mark, 9.0, pal, 0.6);
        push_vgrad(&mut shapes, mark, 9.0, pal.accent_bright, mix(pal.accent2, pal.accent, 0.5), 4);
        shapes.push(Shape::rect_stroke(mark, Rounding::same(9.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.6))));
        p.add(Shape::Vec(shapes));
        let c = mark.center();
        p.circle_stroke(c, 6.6, Stroke::new(1.4_f32, Color32::WHITE));
        for (dx, dy) in [(0.0_f32, -1.0_f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
            p.line_segment(
                [Pos2::new(c.x + dx * 4.6, c.y + dy * 4.6), Pos2::new(c.x + dx * 9.4, c.y + dy * 9.4)],
                Stroke::new(1.4_f32, Color32::WHITE),
            );
        }
        p.text(
            Pos2::new(mark.right() + 11.0, cy - 6.0),
            Align2::LEFT_CENTER,
            t(lang, "title"),
            FontId::proportional(15.0),
            pal.text,
        );
        p.text(
            Pos2::new(mark.right() + 11.0, cy + 10.0),
            Align2::LEFT_CENTER,
            format!("{} · v{}", t(lang, "theme"), crate::VERSION),
            FontId::proportional(10.0),
            pal.dim,
        );

        // 右侧控件（从右往左：关闭 / 最小化 / 最大化 / 主题 / 语言）
        let ctrl_rect = Rect::from_min_max(Pos2::new(rect.right() - ctrl_w, rect.top() + 4.0), Pos2::new(rect.right(), rect.bottom() - 4.0));
        let mut cui = ui.new_child(UiBuilder::new().max_rect(ctrl_rect).layout(Layout::right_to_left(Align::Center)));
        cui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
        let close = win_button(&mut cui, pal, "win_close", WinBtn::Close);
        let min = win_button(&mut cui, pal, "win_min", WinBtn::Minimize);
        let max = win_button(&mut cui, pal, "win_max", WinBtn::Maximize(maximized));
        // 主题 / 语言
        let theme_text = match self.theme.as_str() {
            "light" => t(lang, "theme_light"),
            "dark" => t(lang, "theme_dark"),
            _ => t(lang, "theme_auto"),
        };
        let mut theme_change: Option<String> = None;
        combo(&mut cui, 74.0, "theme_bar", theme_text.to_string(), |ui| {
            for (val, key) in [("auto", "theme_auto"), ("light", "theme_light"), ("dark", "theme_dark")] {
                if ui.selectable_label(theme_text == t(lang, key), t(lang, key)).clicked() {
                    theme_change = Some(val.to_string());
                }
            }
        });
        let mut lang_change: Option<Lang> = None;
        combo(
            &mut cui,
            58.0,
            "lang_bar",
            match lang {
                Lang::Zh => "中文".to_string(),
                Lang::En => "EN".to_string(),
            },
            |ui| {
                if ui.selectable_label(lang == Lang::Zh, "中文").clicked() {
                    lang_change = Some(Lang::Zh);
                }
                if ui.selectable_label(lang == Lang::En, "English").clicked() {
                    lang_change = Some(Lang::En);
                }
            },
        );
        if close.clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
        if min.clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        if max.clicked() {
            ui.ctx().send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }
        let mut persist = false;
        if let Some(v) = theme_change {
            self.theme = v;
            persist = true;
        }
        if let Some(v) = lang_change {
            self.lang = v;
            persist = true;
        }
        if persist {
            let mut store = self.store.lock();
            store.app.language = self.lang.code().to_string();
            store.app.theme = self.theme.clone();
            let _ = store.save_app();
        }
    }

    /// 左侧导航：玻璃胶囊 + accent 光条 + 几何图标
    fn nav_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        ui.add_space(4.0);
        for (idx, key) in NAV_ITEMS {
            let selected = self.active_section == idx;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
            let hot = ui.ctx().animate_bool_with_time(Id::new(("nav_h", idx)), resp.hovered(), 0.12);
            let sel = ui.ctx().animate_value_with_time(Id::new(("nav_s", idx)), if selected { 1.0 } else { 0.0 }, 0.16);
            let r = Rect::from_min_max(
                Pos2::new(rect.left() + 4.0, rect.top() + 2.0),
                Pos2::new(rect.right() - 6.0, rect.bottom() - 2.0),
            );
            let p = ui.painter();
            if sel > 0.01 {
                p.rect_filled(r, Rounding::same(11.0), fade(pal.control, sel * 1.4));
                p.rect_stroke(r, Rounding::same(11.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.20 * sel)));
                let bar = Rect::from_min_max(
                    Pos2::new(r.left() + 3.0, r.center().y - 8.0 * sel),
                    Pos2::new(r.left() + 5.6, r.center().y + 8.0 * sel),
                );
                p.rect_filled(bar.expand(2.2), Rounding::same(3.0), fade(pal.accent, 0.18 * sel));
                p.rect_filled(bar, Rounding::same(1.4), fade(pal.accent, sel));
            } else if hot > 0.01 {
                p.rect_filled(r, Rounding::same(11.0), fade(pal.control, hot * 0.7));
            }
            let fg = if selected { pal.nav_fg_on } else { mix(pal.nav_fg, pal.nav_fg_on, hot) };
            nav_glyph(p, idx, Pos2::new(r.left() + 22.0, r.center().y), if selected { pal.accent } else { fg });
            p.text(
                Pos2::new(r.left() + 37.0, r.center().y),
                Align2::LEFT_CENTER,
                t(self.lang, key),
                FontId::proportional(12.5),
                fg,
            );
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.clicked() {
                self.active_section = idx;
            }
        }
    }

    /// 底部操作条：连接状态（几何圆点）+ flash 淡出 + 应用 / 退出主程序
    fn bottom_bar(&mut self, ui: &mut Ui, pal: &Palette, rect: Rect) {
        let lang = self.lang;
        let mut shapes = Vec::new();
        push_panel(&mut shapes, rect, R_CARD, pal, 0.0);
        ui.painter().add(Shape::Vec(shapes));

        let inner = rect.shrink2(Vec2::new(16.0, 10.0));
        let mut bui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
        bui.spacing_mut().item_spacing = Vec2::new(8.0, 4.0);

        let (dot, msg, msg_color) = if self.backend_ok {
            if self.dirty {
                (pal.warn, t(lang, "backend_dirty"), pal.warn)
            } else {
                (pal.ok, t(lang, "backend_connected"), pal.label)
            }
        } else {
            (pal.warn, t(lang, "backend_file_mode"), pal.label)
        };
        let (dr, _) = bui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
        let p = bui.painter();
        p.circle_filled(dr.center(), 5.6, fade(dot, 0.20));
        p.circle_filled(dr.center(), 3.2, dot);
        bui.label(RichText::new(msg).size(11.0).color(msg_color));

        let mut expire = false;
        bui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button_sized(ui, pal, "apply", t(lang, "push_apply"), Btn::Primary, 168.0, 30.0, 13.5).clicked() {
                self.push_to_backend();
            }
            if button_sized(ui, pal, "quit_app", t(lang, "quit_backend"), Btn::Glass, 104.0, 30.0, 12.5).clicked() {
                if let Some(appdata) = crate::appdata_dir().ok() {
                    let _ = std::fs::write(appdata.join("cmd.json"), "{\"cmd\":\"quit\"}");
                    self.flash(t(lang, "quit_backend_sent").to_string());
                }
            }
            ui.add_space(4.0);
            if let Some((text, at)) = self.status.as_ref() {
                let el = at.elapsed().as_secs_f32();
                if el < STATUS_TTL.as_secs_f32() {
                    let k = if el > 1.3 { (1.0 - (el - 1.3) / (STATUS_TTL.as_secs_f32() - 1.3)).clamp(0.0, 1.0) } else { 1.0 };
                    ui.label(RichText::new(text.clone()).size(11.0).color(with_alpha(pal.ok, (255.0 * k) as u8)));
                    // 仅在淡出进行中请求重绘（静止时不空转）
                    if k < 1.0 {
                        ui.ctx().request_repaint();
                    }
                } else {
                    expire = true;
                }
            }
        });
        if expire {
            self.status = None;
        }
    }

    // ================================================================
    // Section 内容
    // ================================================================

    fn section_style(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        card(ui, pal, None, |ui| {
            let h = 196.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
            preview::paint_preview(ui, rect, &self.preset, lang);
            ui.horizontal(|ui| {
                ui.label(RichText::new(t(lang, "preview")).size(10.5).color(pal.dim));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(18.0, 18.0), Sense::hover());
                    let (rr, gg, bb) = crate::overlay::parse_hex(&self.preset.color);
                    let p = ui.painter();
                    p.rect_filled(r, Rounding::same(5.0), Color32::from_rgb((rr * 255.0) as u8, (gg * 255.0) as u8, (bb * 255.0) as u8));
                    p.rect_stroke(r, Rounding::same(5.0), Stroke::new(1.0_f32, pal.border));
                    ui.label(
                        RichText::new(format!("{} {:.0} · {:.1}", shape_name(lang, self.preset.shape), self.preset.size, self.preset.thickness))
                            .size(10.5)
                            .color(pal.label),
                    );
                });
            });
        });

        // ---- 模板画廊（8 个缩略图） ----
        card(ui, pal, Some(t(lang, "templates")), |ui| {
            note(ui, pal, t(lang, "templates_hint"));
            let cols = 4usize;
            let gap = 10.0;
            let avail = ui.available_width();
            let cell_w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor().max(80.0);
            for row in 0..TEMPLATES.len().div_ceil(cols) {
                ui.horizontal(|ui| {
                    for col in 0..cols {
                        let i = row * cols + col;
                        if i >= TEMPLATES.len() {
                            break;
                        }
                        let (key, _) = TEMPLATES[i];
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(cell_w, 96.0), Sense::click());
                        let hot = ui.ctx().animate_bool_with_time(Id::new(("tpl", i)), resp.hovered(), 0.14);
                        let c = rect.translate(Vec2::new(0.0, -2.0 * hot));
                        let mut shapes = Vec::new();
                        push_panel(&mut shapes, c, 12.0, pal, hot);
                        shapes.push(Shape::rect_stroke(
                            c,
                            Rounding::same(12.0),
                            Stroke::new(1.0_f32, fade(pal.accent, hot * 0.8)),
                        ));
                        ui.painter().add(Shape::Vec(shapes));
                        let prev = Rect::from_min_max(Pos2::new(c.left() + 5.0, c.top() + 5.0), Pos2::new(c.right() - 5.0, c.top() + 62.0));
                        preview::paint_preview_cells(ui, prev, &self.tpl_previews[i], lang, 16.0);
                        ui.painter().text(
                            Pos2::new(c.center().x, c.bottom() - 17.0),
                            Align2::CENTER_CENTER,
                            ellipsize(t(lang, key), 12),
                            FontId::proportional(11.0),
                            mix(pal.label, pal.text, hot),
                        );
                        if resp.clicked() {
                            (TEMPLATES[i].1)(&mut self.preset);
                            self.sync_buffers();
                            self.dirty = true;
                            self.flash(t(lang, "template_applied").to_string());
                        }
                    }
                });
            }
        });

        // ---- 形状与样式 ----
        card(ui, pal, Some(t(lang, "shape_style")), |ui| {
            ui.horizontal(|ui| {
                field_label(ui, pal, t(lang, "shape"), LABEL_W);
                let w = ui.available_width() - 4.0;
                let cur = shape_name(lang, self.preset.shape);
                let mut picked: Option<CrossShape> = None;
                combo(ui, w.max(140.0), "shape_glass", cur.to_string(), |ui| {
                    for s in CrossShape::ALL {
                        if ui.selectable_label(self.preset.shape == s, shape_name(lang, s)).clicked() {
                            picked = Some(s);
                        }
                    }
                });
                if let Some(s) = picked {
                    self.preset.shape = s;
                    self.dirty = true;
                }
            });
            if slider_row(ui, pal, t(lang, "size"), |ui, w| {
                slider_f32(ui, pal, "size", &mut self.preset.size, Rangef::new(1.0, 200.0), w, 0, "")
            }) {
                self.dirty = true;
            }
            if slider_row(ui, pal, t(lang, "thickness"), |ui, w| {
                slider_f32(ui, pal, "thick", &mut self.preset.thickness, Rangef::new(0.2, 20.0), w, 1, "")
            }) {
                self.dirty = true;
            }
            if slider_row(ui, pal, t(lang, "opacity"), |ui, w| {
                slider_f32(ui, pal, "op", &mut self.preset.opacity, Rangef::new(0.05, 1.0), w, 2, "")
            }) {
                self.dirty = true;
            }
            if slider_row(ui, pal, t(lang, "rotation"), |ui, w| {
                slider_f32(ui, pal, "rot", &mut self.preset.rotation, Rangef::new(0.0, 360.0), w, 0, "°")
            }) {
                self.dirty = true;
            }
            if checkbox(ui, pal, "multicolor", &mut self.preset.multicolor, t(lang, "multicolor")) {
                self.dirty = true;
            }
            ui.add_space(2.0);
            if self.preset.multicolor {
                if color_row_ui(ui, pal, t(lang, "color_top"), &mut self.hex_top, &mut self.preset.colors.top) { self.dirty = true; }
                if color_row_ui(ui, pal, t(lang, "color_bottom"), &mut self.hex_bottom, &mut self.preset.colors.bottom) { self.dirty = true; }
                if color_row_ui(ui, pal, t(lang, "color_left"), &mut self.hex_left, &mut self.preset.colors.left) { self.dirty = true; }
                if color_row_ui(ui, pal, t(lang, "color_right"), &mut self.hex_right, &mut self.preset.colors.right) { self.dirty = true; }
            } else if color_row_ui(ui, pal, t(lang, "main_color"), &mut self.hex_main, &mut self.preset.color) {
                self.dirty = true;
            }

            let has_gap = matches!(
                self.preset.shape,
                CrossShape::HollowCross | CrossShape::HollowSquare | CrossShape::HollowCrossDot | CrossShape::GapHair
            );
            if has_gap {
                ui.add_space(2.0);
                if slider_row(ui, pal, t(lang, "hollow_gap"), |ui, w| {
                    slider_f32(ui, pal, "gap", &mut self.preset.hollow.gap, Rangef::new(0.0, 80.0), w, 0, "")
                }) {
                    self.dirty = true;
                }
                if matches!(self.preset.shape, CrossShape::HollowCrossDot | CrossShape::GapHair)
                    && slider_row(ui, pal, t(lang, "center_dot"), |ui, w| {
                        slider_f32(ui, pal, "cds", &mut self.preset.hollow.center_dot_size, Rangef::new(1.0, 30.0), w, 1, "")
                    })
                {
                    self.dirty = true;
                }
            }

            ui.add_space(4.0);
            if checkbox(ui, pal, "outline", &mut self.preset.outline.enabled, t(lang, "outline")) {
                self.dirty = true;
            }
            if self.preset.outline.enabled {
                if slider_row(ui, pal, t(lang, "outline_thickness"), |ui, w| {
                    slider_f32(ui, pal, "ot", &mut self.preset.outline.thickness, Rangef::new(0.5, 10.0), w, 1, "")
                }) {
                    self.dirty = true;
                }
                if color_row_ui(ui, pal, t(lang, "outline_color"), &mut self.hex_outline, &mut self.preset.outline.color) {
                    self.dirty = true;
                }
                if slider_row(ui, pal, t(lang, "outline_opacity"), |ui, w| {
                    slider_f32(ui, pal, "oo", &mut self.preset.outline.opacity, Rangef::new(0.1, 1.0), w, 2, "")
                }) {
                    self.dirty = true;
                }
            }
        });
    }

    fn section_dynamic(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        card(ui, pal, Some(t(lang, "dynamic")), |ui| {
            if slider_row(ui, pal, t(lang, "fire_expand"), |ui, w| {
                slider_f32(ui, pal, "fe", &mut self.preset.dynamic.fire_expand_px, Rangef::new(0.0, 80.0), w, 0, "px")
            }) {
                self.dirty = true;
            }
            let mut rec = self.preset.dynamic.recover_ms as i32;
            if slider_row(ui, pal, t(lang, "recover_ms"), |ui, w| {
                slider_i32(ui, pal, "rm", &mut rec, Rangef::new(20.0, 500.0), w)
            }) {
                self.preset.dynamic.recover_ms = rec.max(1) as u32;
                self.dirty = true;
            }
            if checkbox(ui, pal, "recoil", &mut self.preset.dynamic.recoil_indicator, t(lang, "recoil_indicator")) {
                self.dirty = true;
            }
        });
    }

    fn section_position(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        // 显示器列表：进入本分区时刷新（3s 内复用，避免每帧枚举）
        let stale = self.monitors_at.map_or(true, |t| t.elapsed() > Duration::from_secs(3));
        if stale {
            self.monitors = crate::system::monitor::monitors();
            self.monitors_at = Some(Instant::now());
        }
        let monitors = self.monitors.clone();
        card(ui, pal, Some(t(lang, "position")), |ui| {
            ui.horizontal(|ui| {
                field_label(ui, pal, t(lang, "monitors_title"), LABEL_W);
                if button_sized(ui, pal, "mon_refresh", t(lang, "monitor_refresh"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                    self.monitors = crate::system::monitor::monitors();
                    self.monitors_at = Some(Instant::now());
                }
            });
            // 跟随前台窗口
            if monitor_row(ui, pal, self.preset.position.monitor == -1, t(lang, "monitor_follow"), None) {
                self.preset.position.monitor = -1;
                self.dirty = true;
            }
            for (i, m) in monitors.iter().enumerate() {
                let w = m.rect.2 - m.rect.0;
                let h = m.rect.3 - m.rect.1;
                let mut label = format!("{} {} · {}×{}", t(lang, "monitor"), i + 1, w, h);
                if m.primary {
                    label.push_str(" · ");
                    label.push_str(t(lang, "monitor_primary"));
                }
                if monitor_row(ui, pal, self.preset.position.monitor == i as i32, &label, Some(m)) {
                    self.preset.position.monitor = i as i32;
                    self.dirty = true;
                }
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if button_sized(ui, pal, "mon_center", t(lang, "center_on_monitor"), Btn::Glass, 0.0, 26.0, 12.0).clicked() {
                    self.preset.position.x = PosVal::Center;
                    self.preset.position.y = PosVal::Center;
                    let sel = self.preset.position.monitor;
                    let target = if sel < 0 {
                        crate::system::monitor::primary().map(|m| m.work_center()).unwrap_or((0, 0))
                    } else {
                        monitors
                            .get(sel as usize)
                            .or_else(|| monitors.first())
                            .map(|m| m.work_center())
                            .unwrap_or((0, 0))
                    };
                    // 用相对该屏的像素值锁死中心（Center 也会跟随主屏/前台窗口，这里显式写点）
                    self.preset.position.x = PosVal::Px(target.0 as f32);
                    self.preset.position.y = PosVal::Px(target.1 as f32);
                    self.dirty = true;
                    self.flash(t(lang, "center_on_monitor").to_string());
                }
                if button_sized(ui, pal, "mon_reset", t(lang, "center_btn"), Btn::Ghost, 0.0, 26.0, 12.0).clicked() {
                    self.preset.position.x = PosVal::Center;
                    self.preset.position.y = PosVal::Center;
                    self.dirty = true;
                }
            });
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                field_label(ui, pal, t(lang, "pos_x"), LABEL_W);
                let mut x = match self.preset.position.x {
                    PosVal::Px(v) => v as i32,
                    PosVal::Center => 0,
                };
                let mut y = match self.preset.position.y {
                    PosVal::Px(v) => v as i32,
                    PosVal::Center => 0,
                };
                let w = (ui.available_width() - 90.0) / 2.0;
                let mut changed = false;
                ui.scope(|ui| {
                    apply_style(ui, pal);
                    if ui.add_sized(Vec2::new(w.max(70.0), 24.0), egui::DragValue::new(&mut x).speed(2.0)).changed() {
                        changed = true;
                    }
                    ui.label(RichText::new(t(lang, "pos_y")).size(12.0).color(pal.label));
                    if ui.add_sized(Vec2::new(w.max(70.0), 24.0), egui::DragValue::new(&mut y).speed(2.0)).changed() {
                        changed = true;
                    }
                });
                if changed {
                    self.preset.position.x = PosVal::Px(x as f32);
                    self.preset.position.y = PosVal::Px(y as f32);
                    self.dirty = true;
                }
            });
            if checkbox(ui, pal, "snap", &mut self.preset.snap_to_window, t(lang, "snap_to_window")) {
                self.dirty = true;
            }
            note(ui, pal, t(lang, "snap_note"));
        });
    }

    fn section_gamepad(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        card(ui, pal, Some(t(lang, "gamepad")), |ui| {
            ui.horizontal(|ui| {
                field_label(ui, pal, t(lang, "ads_mode"), LABEL_W);
                let mut picked: Option<AdsMode> = None;
                combo(ui, 130.0, "ads_mode_glass", ads_mode_name(lang, self.preset.gamepad.ads_mode).to_string(), |ui| {
                    for m in [AdsMode::Off, AdsMode::HoldHide, AdsMode::Toggle, AdsMode::HoldShow] {
                        if ui.selectable_label(self.preset.gamepad.ads_mode == m, ads_mode_name(lang, m)).clicked() {
                            picked = Some(m);
                        }
                    }
                });
                if let Some(m) = picked {
                    self.preset.gamepad.ads_mode = m;
                    self.dirty = true;
                }
            });
            ui.horizontal(|ui| {
                field_label(ui, pal, t(lang, "ads_button"), LABEL_W);
                let cur = match self.preset.gamepad.ads_button {
                    AdsButton::LeftTrigger => t(lang, "ads_left_trigger"),
                    AdsButton::RightTrigger => t(lang, "ads_right_trigger"),
                    AdsButton::LeftBumper => t(lang, "ads_left_bumper"),
                    AdsButton::RightBumper => t(lang, "ads_right_bumper"),
                };
                let mut picked: Option<AdsButton> = None;
                combo(ui, 170.0, "ads_btn_glass", cur.to_string(), |ui| {
                    for b in [AdsButton::LeftTrigger, AdsButton::RightTrigger, AdsButton::LeftBumper, AdsButton::RightBumper] {
                        if ui.selectable_label(self.preset.gamepad.ads_button == b, ads_button_name(lang, b)).clicked() {
                            picked = Some(b);
                        }
                    }
                });
                if let Some(b) = picked {
                    self.preset.gamepad.ads_button = b;
                    self.dirty = true;
                }
            });
            let mut thr = self.preset.gamepad.trigger_threshold as i32;
            if slider_row(ui, pal, t(lang, "trigger_threshold"), |ui, w| {
                slider_i32(ui, pal, "thr", &mut thr, Rangef::new(0.0, 255.0), w)
            }) {
                self.preset.gamepad.trigger_threshold = thr.clamp(0, 255) as u8;
                self.dirty = true;
            }
            if checkbox(ui, pal, "gp_fire", &mut self.preset.gamepad.fire_expand, t(lang, "gamepad_fire_expand")) {
                self.dirty = true;
            }
            note(ui, pal, t(lang, "gamepad_note"));
        });
    }

    fn section_hotkey(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        card(ui, pal, Some(t(lang, "hotkey")), |ui| {
            let toggle = self.preset.hotkey_toggle;
            self.hotkey_field(ui, pal, 0, t(lang, "hotkey_toggle"), toggle);
            let next = self.preset.hotkey_next_profile;
            self.hotkey_field(ui, pal, 1, t(lang, "hotkey_next"), next);
            note(ui, pal, t(lang, "hotkey_note"));
            ui.add_space(4.0);
            if checkbox(ui, pal, "rc_toggle", &mut self.preset.right_click_toggle, t(lang, "right_click")) {
                self.dirty = true;
            }
            if self.preset.right_click_toggle {
                ui.horizontal(|ui| {
                    field_label(ui, pal, t(lang, "right_click_mode"), LABEL_W);
                    let cur = match self.preset.right_click_mode {
                        RightClickMode::Click => t(lang, "rc_click"),
                        RightClickMode::HoldShow => t(lang, "rc_hold_show"),
                        RightClickMode::HoldHide => t(lang, "rc_hold_hide"),
                    };
                    let mut picked: Option<RightClickMode> = None;
                    combo(ui, 140.0, "rc_mode_glass", cur.to_string(), |ui| {
                        for (m, k) in [
                            (RightClickMode::Click, "rc_click"),
                            (RightClickMode::HoldShow, "rc_hold_show"),
                            (RightClickMode::HoldHide, "rc_hold_hide"),
                        ] {
                            if ui.selectable_label(self.preset.right_click_mode == m, t(lang, k)).clicked() {
                                picked = Some(m);
                            }
                        }
                    });
                    if let Some(m) = picked {
                        self.preset.right_click_mode = m;
                        self.dirty = true;
                    }
                });
            }
        });
    }

    /// 单个热键：录制框 + 清空 + 手工输入（保留旧版直接输入字符串的能力）
    fn hotkey_field(&mut self, ui: &mut Ui, pal: &Palette, slot: usize, label: &str, hk: Hotkey) {
        let lang = self.lang;
        let recording = self.recording == Some(slot);
        ui.horizontal(|ui| {
            field_label(ui, pal, label, LABEL_W);
            let avail = ui.available_width();
            let text = if recording {
                t(lang, "hotkey_recording").to_string()
            } else if hk.is_empty() {
                t(lang, "hotkey_record").to_string()
            } else {
                hk.to_string()
            };
            let resp = key_box(ui, pal, slot, &text, (avail - 104.0).max(140.0), recording, hk.is_empty() && !recording);
            if resp.clicked() {
                self.recording = if recording { None } else { Some(slot) };
                if self.recording.is_some() {
                    // 录制定按键：先交出文本框焦点，否则 Backspace/Delete 会同时删文本
                    ui.ctx().memory_mut(|m| m.stop_text_input());
                }
            }
            if button_sized(ui, pal, &format!("hk_clr{slot}"), t(lang, "hotkey_clear"), Btn::Ghost, 0.0, 24.0, 11.0).clicked() {
                match slot {
                    0 => self.preset.hotkey_toggle = Hotkey::default(),
                    _ => self.preset.hotkey_next_profile = Hotkey::default(),
                }
                self.sync_buffers();
                self.recording = None;
                self.dirty = true;
            }
        });
        ui.horizontal(|ui| {
            field_label(ui, pal, t(lang, "hotkey_manual"), LABEL_W);
            let w = ui.available_width() - 4.0;
            let changed = if slot == 0 {
                let r = text_field(ui, pal, &mut self.hotkey_buf, w, "Ctrl+F1");
                r.changed()
            } else {
                let r = text_field(ui, pal, &mut self.hotkey_next_buf, w, "Ctrl+F2");
                r.changed()
            };
            if changed {
                let hk = Hotkey::parse(if slot == 0 { &self.hotkey_buf } else { &self.hotkey_next_buf }).unwrap_or_default();
                match slot {
                    0 => self.preset.hotkey_toggle = hk,
                    _ => self.preset.hotkey_next_profile = hk,
                }
                self.dirty = true;
            }
        });
    }

    fn section_image(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        // 缩略图按路径缓存（只在路径变化时解码一次）
        if self.thumb_for != self.preset.image.path {
            self.thumb_for = self.preset.image.path.clone();
            self.thumb = Thumb::Empty;
            if !self.preset.image.path.is_empty() && std::path::Path::new(&self.preset.image.path).is_file() {
                match image::open(&self.preset.image.path) {
                    Ok(img) => {
                        let small = img.thumbnail(160, 160).to_rgba8();
                        let (w, h) = small.dimensions();
                        let ci = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], small.as_raw());
                        self.thumb = Thumb::Ready(ui.ctx().load_texture("acaja_img_thumb", ci, egui::TextureOptions::LINEAR));
                    }
                    Err(e) => {
                        warn!("图片解码失败: {e}");
                        self.thumb = Thumb::Failed;
                    }
                }
            }
        }

        card(ui, pal, Some(t(lang, "custom_image")), |ui| {
            ui.horizontal(|ui| {
                field_label(ui, pal, t(lang, "image_path"), LABEL_W);
                let text = if self.preset.image.path.is_empty() {
                    t(lang, "image_none").to_string()
                } else {
                    file_name(&self.preset.image.path)
                };
                let w = (ui.available_width() - 190.0).max(90.0);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::hover());
                let mut shapes = Vec::new();
                push_well(&mut shapes, rect, 8.0, pal);
                ui.painter().add(Shape::Vec(shapes));
                ui.painter().text(
                    Pos2::new(rect.left() + 9.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    ellipsize(&text, 34),
                    FontId::proportional(11.5),
                    if self.preset.image.path.is_empty() { pal.dim } else { pal.text },
                );
                let tip = if self.preset.image.path.is_empty() {
                    String::new()
                } else {
                    self.preset.image.path.clone()
                };
                if !tip.is_empty() {
                    resp.on_hover_text(tip);
                }
                if button_sized(ui, pal, "img_pick", t(lang, "image_choose"), Btn::Glass, 0.0, 26.0, 11.5).clicked() {
                    if let Some(p) = crate::system::filedialog::pick_image() {
                        self.preset.image.path = p.to_string_lossy().into_owned();
                        self.dirty = true;
                    }
                }
                if button_sized(ui, pal, "img_clear", t(lang, "clear"), Btn::Ghost, 0.0, 26.0, 11.5).clicked() {
                    self.preset.image.path.clear();
                    self.dirty = true;
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                // 缩略图井
                let (well, _) = ui.allocate_exact_size(Vec2::new(132.0, 104.0), Sense::hover());
                let mut shapes = Vec::new();
                push_well(&mut shapes, well, 12.0, pal);
                ui.painter().add(Shape::Vec(shapes));
                let inner = well.shrink(6.0);
                match &self.thumb {
                    Thumb::Ready(tex) => {
                        ui.painter().image(
                            tex.id(),
                            inner,
                            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    Thumb::Failed => {
                        ui.painter().text(inner.center(), Align2::CENTER_CENTER, t(lang, "image_unreadable"), FontId::proportional(11.0), pal.danger);
                    }
                    Thumb::Empty => {
                        ui.painter().text(inner.center(), Align2::CENTER_CENTER, t(lang, "image_preview"), FontId::proportional(11.0), pal.dim);
                    }
                }
                ui.vertical(|ui| {
                    if slider_row(ui, pal, t(lang, "image_scale"), |ui, w| {
                        slider_f32(ui, pal, "isc", &mut self.preset.image.scale, Rangef::new(0.1, 5.0), w, 2, "x")
                    }) {
                        self.dirty = true;
                    }
                    if self.preset.image.path.is_empty() {
                        note(ui, pal, t(lang, "image_none"));
                    }
                });
            });
        });
    }

    fn section_presets(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        let names = { self.store.lock().preset_names() };

        card(ui, pal, Some(t(lang, "presets")), |ui| {
            ui.horizontal(|ui| {
                if button_sized(ui, pal, "p_save", t(lang, "save"), Btn::Glass, 0.0, 26.0, 12.0).clicked() {
                    self.save_current();
                }
                if button_sized(ui, pal, "p_import", t(lang, "import"), Btn::Glass, 0.0, 26.0, 12.0).clicked() {
                    if let Some(path) = crate::system::filedialog::pick_preset_json() {
                        let res = { self.store.lock().import_preset(&path, None) };
                        match res {
                            Ok(name) => {
                                self.apply_preset(&name);
                                self.flash(format!("{} {}", t(lang, "imported"), name));
                            }
                            Err(e) => {
                                warn!("导入预设失败: {e}");
                                self.flash(format!("{}: {}", t(lang, "import_failed"), e));
                            }
                        }
                    }
                }
                ui.add_space(6.0);
                ui.add(TextEdit::singleline(&mut self.new_preset_name).desired_width(150.0).hint_text(t(lang, "new_name")));
                if button_sized(ui, pal, "p_create", t(lang, "create"), Btn::Glass, 0.0, 26.0, 12.0).clicked() {
                    let name = self.new_preset_name.trim().to_string();
                    match crate::config::validate_preset_name(&name) {
                        Err(e) => self.flash(e.to_string()),
                        Ok(()) => {
                            let result = {
                                let mut store = self.store.lock();
                                let res = store.save_preset(&name, &self.preset.clone());
                                if res.is_ok() {
                                    let _ = store.save_app();
                                }
                                res
                            };
                            match result {
                                Ok(()) => {
                                    self.active_name = name;
                                    self.new_preset_name.clear();
                                    self.flash(format!("{} {}", t(lang, "created"), self.active_name));
                                }
                                Err(e) => self.flash(format!("{}: {e}", t(lang, "error"))),
                            }
                        }
                    }
                }
            });
            note(ui, pal, t(lang, "new_preset"));
            ui.add_space(2.0);

            // ---- 预设列表 ----
            for name in &names {
                let active = *name == self.active_name;
                let editing = self.rename_from.as_deref() == Some(name.as_str());
                let confirming = self.delete_confirm.as_deref() == Some(name.as_str());
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), Sense::click());
                let hot = ui.ctx().animate_bool_with_time(Id::new(("prow", name)), resp.hovered(), 0.12);
                let r = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 1.0), Pos2::new(rect.right(), rect.bottom() - 1.0));
                let p = ui.painter();
                if active {
                    p.rect_filled(r, Rounding::same(11.0), pal.accent_soft);
                    p.rect_stroke(r, Rounding::same(11.0), Stroke::new(1.0_f32, fade(pal.accent, 0.45)));
                    p.rect_filled(
                        Rect::from_min_max(Pos2::new(r.left() + 3.0, r.center().y - 9.0), Pos2::new(r.left() + 5.4, r.center().y + 9.0)),
                        Rounding::same(1.4),
                        pal.accent,
                    );
                } else if hot > 0.01 {
                    p.rect_filled(r, Rounding::same(11.0), fade(pal.control, hot * 0.8));
                }
                p.circle_filled(Pos2::new(r.left() + 19.0, r.center().y), 3.2, if active { pal.ok } else { pal.dim });
                // 名称裁到自己的列内：窗口缩到最小宽度也不会压到右侧按钮
                let name_rect = Rect::from_min_max(
                    Pos2::new(r.left() + 31.0, r.top()),
                    Pos2::new((r.right() - 364.0).max(r.left() + 60.0), r.bottom()),
                );
                let np = p.with_clip_rect(name_rect.intersect(p.clip_rect()));
                np.text(
                    Pos2::new(r.left() + 31.0, r.center().y),
                    Align2::LEFT_CENTER,
                    ellipsize(name, 24),
                    FontId::proportional(12.5),
                    if active { pal.text } else { pal.label },
                );
                if resp.clicked() && !active && !editing && !confirming {
                    self.apply_preset(name);
                }

                // 行内操作按钮
                let acts = Rect::from_min_max(Pos2::new(r.right() - 356.0, r.top() + 3.0), Pos2::new(r.right() - 6.0, r.bottom() - 3.0));
                let mut aui = ui.new_child(UiBuilder::new().max_rect(acts).layout(Layout::right_to_left(Align::Center)));
                aui.spacing_mut().item_spacing = Vec2::new(4.0, 2.0);
                let mut del_now = false;
                let mut dup = false;
                let mut ren = false;
                let mut exp = false;
                if confirming {
                    if button_sized(&mut aui, pal, &format!("pc_no{name}"), t(lang, "cancel"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        self.delete_confirm = None;
                    }
                    if button_sized(&mut aui, pal, &format!("pc_yes{name}"), t(lang, "confirm"), Btn::Danger, 0.0, 22.0, 11.0).clicked() {
                        del_now = true;
                    }
                    aui.label(RichText::new(format!("{} {}?", t(lang, "delete_confirm"), ellipsize(name, 12))).size(11.0).color(pal.danger));
                } else {
                    if button_sized(&mut aui, pal, &format!("pc_del{name}"), t(lang, "btn_delete"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        if *name == "default" {
                            self.flash(t(lang, "cannot_delete_default").to_string());
                        } else {
                            self.delete_confirm = Some(name.clone());
                        }
                    }
                    if button_sized(&mut aui, pal, &format!("pc_exp{name}"), t(lang, "export"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        exp = true;
                    }
                    if button_sized(&mut aui, pal, &format!("pc_ren{name}"), t(lang, "rename"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        ren = true;
                    }
                    if button_sized(&mut aui, pal, &format!("pc_dup{name}"), t(lang, "duplicate"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        dup = true;
                    }
                    if !active && button_sized(&mut aui, pal, &format!("pc_use{name}"), t(lang, "activate"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        self.apply_preset(name);
                    }
                }
                if dup {
                    let res = { self.store.lock().duplicate_preset(name) };
                    match res {
                        Ok(new_name) => {
                            self.apply_preset(&new_name);
                            self.flash(format!("{} {}", t(lang, "duplicate"), new_name));
                        }
                        Err(e) => {
                            warn!("复制预设失败: {e}");
                            self.flash(format!("{}: {e}", t(lang, "duplicate_failed")));
                        }
                    }
                }
                if ren {
                    self.rename_from = Some(name.clone());
                    self.rename_buf = name.clone();
                    self.rename_confirm = false;
                    self.delete_confirm = None;
                }
                if exp {
                    if let Some(path) = crate::system::filedialog::save_preset_json(name) {
                        let res = { self.store.lock().export_preset(name, &path) };
                        match res {
                            Ok(()) => self.flash(format!("{} {}", t(lang, "exported"), file_name(&path.to_string_lossy()))),
                            Err(e) => {
                                warn!("导出预设失败: {e}");
                                self.flash(format!("{}: {e}", t(lang, "export_failed")));
                            }
                        }
                    }
                }
                if del_now {
                    let (ok, deleted) = {
                        let mut store = self.store.lock();
                        let ok = store.delete_preset(name).unwrap_or(false);
                        if ok {
                            let _ = store.save_app();
                        }
                        (ok, name.clone())
                    };
                    self.delete_confirm = None;
                    if ok {
                        self.flash(format!("{} {}", t(lang, "deleted"), deleted));
                        if self.active_name == deleted {
                            self.apply_preset("default");
                        }
                    }
                }

                // 内联重命名（实时校验）
                if editing {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(30.0);
                        ui.add(TextEdit::singleline(&mut self.rename_buf).desired_width(200.0));
                        let trimmed = self.rename_buf.trim().to_string();
                        let taken = names.iter().any(|n| n.as_str() == trimmed);
                        let name_ok = crate::config::validate_preset_name(&self.rename_buf).is_ok()
                            && trimmed != *name
                            && !taken;
                        if self.rename_confirm {
                            if button_sized(ui, pal, "pr_yes", t(lang, "confirm"), Btn::Primary, 0.0, 24.0, 12.0).clicked() {
                                let to = self.rename_buf.trim().to_string();
                                let from = name.clone();
                                let res = { self.store.lock().rename_preset(&from, &to) };
                                self.rename_confirm = false;
                                match res {
                                    Ok(true) => {
                                        self.rename_from = None;
                                        if self.active_name == from {
                                            self.active_name = to.clone();
                                        }
                                        self.flash(format!("{} {}", t(lang, "renamed"), to));
                                    }
                                    Ok(false) => self.flash(t(lang, "rename_failed").to_string()),
                                    Err(e) => {
                                        warn!("重命名失败: {e}");
                                        self.flash(format!("{}: {e}", t(lang, "rename_failed")));
                                    }
                                }
                            }
                            if button_sized(ui, pal, "pr_no", t(lang, "cancel"), Btn::Ghost, 0.0, 24.0, 12.0).clicked() {
                                self.rename_confirm = false;
                            }
                            ui.label(
                                RichText::new(format!("{} {}?", t(lang, "rename_confirm"), ellipsize(&self.rename_buf, 14)))
                                    .size(11.0)
                                    .color(pal.warn),
                            );
                        } else {
                            if button_sized(ui, pal, "pr_go", t(lang, "rename"), Btn::Glass, 0.0, 24.0, 12.0).clicked() && name_ok {
                                self.rename_confirm = true;
                            }
                            if button_sized(ui, pal, "pr_cancel", t(lang, "cancel"), Btn::Ghost, 0.0, 24.0, 12.0).clicked() {
                                self.rename_from = None;
                            }
                            match crate::config::validate_preset_name(&self.rename_buf) {
                                Err(e) => {
                                    ui.label(RichText::new(e).size(11.0).color(pal.danger));
                                }
                                Ok(()) if trimmed == *name => {
                                    ui.label(RichText::new(t(lang, "rename")).size(11.0).color(pal.dim));
                                }
                                Ok(()) if taken => {
                                    ui.label(RichText::new(t(lang, "rename_failed")).size(11.0).color(pal.warn));
                                }
                                Ok(()) => {}
                            }
                        }
                    });
                    ui.add_space(2.0);
                }
            }
        });

        // ---- 游戏绑定：前台进程 → 自动套用预设 ----
        card(ui, pal, Some(t(lang, "bindings")), |ui| {
            let bindings: Vec<(String, String)> = {
                let store = self.store.lock();
                store.app.game_bindings.iter().map(|b| (b.exe.clone(), b.preset.clone())).collect()
            };
            if bindings.is_empty() {
                ui.label(RichText::new(t(lang, "binding_none")).size(11.0).color(pal.dim));
            }
            let mut remove: Option<usize> = None;
            for (i, (exe, preset)) in bindings.iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
                ui.painter().rect_filled(rect, Rounding::same(9.0), fade(pal.control, 0.55));
                ui.painter().rect_stroke(rect, Rounding::same(9.0), Stroke::new(1.0_f32, pal.border));
                ui.painter().text(Pos2::new(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, exe, FontId::proportional(11.5), pal.text);
                ui.painter().text(Pos2::new(rect.right() - 84.0, rect.center().y), Align2::RIGHT_CENTER, preset, FontId::proportional(11.5), pal.accent_bright);
                let brect = Rect::from_min_max(Pos2::new(rect.right() - 76.0, rect.top() + 3.0), Pos2::new(rect.right() - 6.0, rect.bottom() - 3.0));
                let mut bui = ui.new_child(UiBuilder::new().max_rect(brect).layout(Layout::right_to_left(Align::Center)));
                if button_sized(&mut bui, pal, &format!("b_rm{i}"), t(lang, "binding_remove"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                    remove = Some(i);
                }
            }
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 320.0).max(110.0);
                ui.add(TextEdit::singleline(&mut self.new_binding_exe).hint_text(t(lang, "binding_exe")).desired_width(w));
                let names = { self.store.lock().preset_names() };
                let mut picked: Option<String> = None;
                combo(ui, 130.0, "bind_preset_glass", self.new_binding_preset.clone(), |ui| {
                    for n in &names {
                        if ui.selectable_label(&self.new_binding_preset == n, n.clone()).clicked() {
                            picked = Some(n.clone());
                        }
                    }
                });
                if let Some(n) = picked {
                    self.new_binding_preset = n;
                }
                if button_sized(ui, pal, "b_add", t(lang, "binding_add"), Btn::Glass, 0.0, 26.0, 12.0).clicked() {
                    let exe = self.new_binding_exe.trim().to_ascii_lowercase();
                    if !exe.is_empty() {
                        let preset = if self.new_binding_preset.is_empty() {
                            self.active_name.clone()
                        } else {
                            self.new_binding_preset.clone()
                        };
                        let saved = {
                            let mut store = self.store.lock();
                            store.app.game_bindings.retain(|b| !b.exe.eq_ignore_ascii_case(&exe));
                            store.app.game_bindings.push(GameBinding { exe: exe.clone(), preset });
                            store.save_app().is_ok()
                        };
                        self.new_binding_exe.clear();
                        self.flash(if saved {
                            t(lang, "binding_added").to_string()
                        } else {
                            t(lang, "error").to_string()
                        });
                    }
                }
            });
            if let Some(i) = remove {
                let saved = {
                    let mut store = self.store.lock();
                    if i < store.app.game_bindings.len() {
                        store.app.game_bindings.remove(i);
                    }
                    store.save_app().is_ok()
                };
                if saved {
                    self.flash(t(lang, "binding_removed").to_string());
                }
            }
            note(ui, pal, t(lang, "binding_note"));
        });
    }

    /// 系统：应用级开关（开机自启）
    fn section_system(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        card(ui, pal, Some(t(lang, "system")), |ui| {
            let mut autostart = { self.store.lock().app.autostart };
            if checkbox(ui, pal, "autostart", &mut autostart, t(lang, "autostart")) {
                // 注册表必须指向**主程序**（acaja.exe）的真实路径：路径从主进程
                // 窗口反查得到，所以程序被改名/移动也不会写错目标。
                // 主程序未运行时无法确定路径 → 不写注册表。
                let target = crate::ipc::find_backend().and_then(crate::system::foreground::window_process_path);
                let applied = match &target {
                    Some(path) => match crate::system::autostart::set_autostart(autostart, std::path::Path::new(path)) {
                        Ok(()) => true,
                        Err(e) => {
                            warn!("开机自启写入注册表失败: {e}");
                            false
                        }
                    },
                    None => {
                        warn!("未找到主程序窗口，无法确定自启路径");
                        false
                    }
                };
                {
                    // 失败则回滚开关，避免「界面显示已开启、注册表里其实没有」
                    let mut store = self.store.lock();
                    store.app.autostart = if applied { autostart } else { !autostart };
                    let _ = store.save_app();
                }
                self.flash(if applied {
                    t(lang, "saved").to_string()
                } else {
                    t(lang, "autostart_failed").to_string()
                });
            }
            note(ui, pal, t(lang, "autostart_note"));
        });
    }

    /// 热键录制：Esc 取消、Backspace/Delete 清空、其余组合键落库
    fn poll_hotkey_recording(&mut self, ctx: &Context) {
        let Some(slot) = self.recording else { return };
        let mods = ctx.input(|i| i.modifiers);
        let win_down = win_key_down();
        let mut captured: Option<Hotkey> = None;
        let mut cancel = false;
        let mut clear = false;
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, .. } = ev {
                    match key {
                        egui::Key::Escape => cancel = true,
                        egui::Key::Backspace | egui::Key::Delete => clear = true,
                        other => {
                            if let Some(vk) = key_to_vk(*other) {
                                let mut m = 0u32;
                                if mods.ctrl {
                                    m |= MOD_CONTROL;
                                }
                                if mods.alt {
                                    m |= MOD_ALT;
                                }
                                if mods.shift {
                                    m |= MOD_SHIFT;
                                }
                                if win_down {
                                    m |= MOD_WIN;
                                }
                                captured = Some(Hotkey { modifiers: m, vk });
                            }
                        }
                    }
                }
            }
        });
        if cancel {
            self.recording = None;
            self.flash(t(self.lang, "cancel").to_string());
            return;
        }
        if clear {
            captured = Some(Hotkey::default());
        }
        if let Some(hk) = captured {
            match slot {
                0 => self.preset.hotkey_toggle = hk,
                _ => self.preset.hotkey_next_profile = hk,
            }
            self.sync_buffers();
            self.recording = None;
            self.dirty = true;
            let text = if hk.is_empty() { t(self.lang, "hotkey_clear").to_string() } else { hk.to_string() };
            self.flash(format!("{}: {}", t(self.lang, "hotkey"), text));
        }
    }
}

/// 滑杆行：标签列（104px）+ 占满剩余宽度的滑杆
fn slider_row(ui: &mut Ui, pal: &Palette, label: &str, body: impl FnOnce(&mut Ui, f32) -> bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        field_label(ui, pal, label, LABEL_W);
        changed = body(ui, ui.available_width() - 4.0);
    });
    changed
}

/// 显示器行：玻璃行 + 选中态 + 分辨率说明
fn monitor_row(ui: &mut Ui, pal: &Palette, selected: bool, label: &str, mon: Option<&MonitorInfo>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    let hot = ui.ctx().animate_bool_with_time(Id::new(("mon", label)), resp.hovered(), 0.12);
    let sel = ui.ctx().animate_value_with_time(Id::new(("monsel", label)), if selected { 1.0 } else { 0.0 }, 0.16);
    let r = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 1.0), Pos2::new(rect.right(), rect.bottom() - 1.0));
    let p = ui.painter();
    p.rect_filled(r, Rounding::same(10.0), pal.control);
    if sel > 0.01 {
        p.rect_filled(r, Rounding::same(10.0), fade(pal.accent_soft, sel * 1.2));
        p.rect_stroke(r, Rounding::same(10.0), Stroke::new(1.0_f32, fade(pal.accent, 0.5 * sel)));
    } else if hot > 0.01 {
        p.rect_filled(r, Rounding::same(10.0), fade(pal.control, hot));
    }
    // 单选圆
    let c = Pos2::new(r.left() + 18.0, r.center().y);
    p.circle_stroke(c, 6.4, Stroke::new(1.4_f32, mix(pal.dim, pal.accent, sel.max(hot * 0.5))));
    if sel > 0.01 {
        p.circle_filled(c, 3.4 * sel, fade(pal.accent, sel));
    }
    p.text(Pos2::new(r.left() + 34.0, r.center().y), Align2::LEFT_CENTER, ellipsize(label, 42), FontId::proportional(12.0), if selected { pal.text } else { pal.label });
    if let Some(m) = mon {
        if m.primary {
            p.circle_filled(Pos2::new(r.right() - 14.0, r.center().y), 3.0, pal.warm);
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp.clicked()
}

/// 热键录制框（点击进入录制态；录制中显示脉冲红点）
fn key_box(ui: &mut Ui, pal: &Palette, slot: usize, text: &str, width: f32, recording: bool, hint: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let hot = ui.ctx().animate_bool_with_time(Id::new(("kbox", slot)), resp.hovered(), 0.12);
    let rec = ui.ctx().animate_bool_with_time(Id::new(("krec", slot)), recording, 0.15);
    let mut shapes = Vec::new();
    push_well(&mut shapes, rect, 8.0, pal);
    if rec > 0.01 {
        shapes.push(Shape::rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0_f32, fade(pal.danger, 0.75 * rec))));
        if recording {
            ui.ctx().request_repaint();
        }
    } else {
        shapes.push(Shape::rect_stroke(rect, Rounding::same(8.0), Stroke::new(1.0_f32, mix(pal.border, pal.accent_bright, hot * 0.7))));
    }
    ui.painter().add(Shape::Vec(shapes));
    if recording {
        let t = ui.input(|i| i.time) as f32;
        let pulse = 0.45 + 0.55 * (t * 4.0).sin().abs();
        let c = Pos2::new(rect.left() + 14.0, rect.center().y);
        ui.painter().circle_filled(c, 5.0, fade(pal.danger, 0.22 * pulse));
        ui.painter().circle_filled(c, 3.2, fade(pal.danger, 0.55 + 0.45 * pulse));
        ui.painter().text(
            Pos2::new(c.x + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(11.5),
            pal.warn,
        );
    } else {
        ui.painter().text(
            Pos2::new(rect.left() + 11.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(12.0),
            if hint { pal.dim } else { pal.text },
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// 颜色编辑行：色块 + hex 输入（返回是否有改动）
fn color_row_ui(ui: &mut Ui, pal: &Palette, label: &str, buf: &mut String, target: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        field_label(ui, pal, label, LABEL_W);
        let (r, g, b) = crate::overlay::parse_hex(target);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(24.0, 24.0), Sense::hover());
        let mut shapes = Vec::new();
        push_well(&mut shapes, rect, 7.0, pal);
        ui.painter().add(Shape::Vec(shapes));
        let inner = rect.shrink(3.0);
        ui.painter().rect_filled(inner, Rounding::same(5.0), Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
        ui.painter().rect_stroke(inner, Rounding::same(5.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.5)));
        let w = (ui.available_width() - 4.0).min(150.0);
        if text_field(ui, pal, buf, w, "#RRGGBB").changed() {
            let s = buf.trim().trim_start_matches('#');
            if s.len() == 6 && u32::from_str_radix(s, 16).is_ok() {
                *target = format!("#{}", s.to_uppercase());
                changed = true;
            }
        }
    });
    changed
}

// ===========================================================================
// 窗口按钮 / 导航图标（纯几何绘制，无 emoji）
// ===========================================================================

enum WinBtn {
    Minimize,
    Maximize(bool),
    Close,
}

fn win_button(ui: &mut Ui, pal: &Palette, seed: &str, kind: WinBtn) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    let hot = ui.ctx().animate_bool_with_time(Id::new(("win", seed)), resp.hovered(), 0.12);
    let danger = matches!(kind, WinBtn::Close);
    let bg = if danger { mix(pal.control, pal.danger, hot) } else { mix(pal.control, pal.hover, hot) };
    if hot > 0.01 {
        ui.painter().rect_filled(rect, Rounding::same(8.0), fade(bg, hot));
    }
    let fg = if danger && hot > 0.5 { Color32::WHITE } else { mix(pal.label, pal.text, hot) };
    let c = rect.center();
    let s = Stroke::new(1.3_f32, fg);
    match kind {
        WinBtn::Minimize => {
            ui.painter().line_segment([Pos2::new(c.x - 5.0, c.y + 3.0), Pos2::new(c.x + 5.0, c.y + 3.0)], s);
        }
        WinBtn::Maximize(maxed) => {
            if maxed {
                let a = Rect::from_min_max(Pos2::new(c.x - 5.5, c.y - 3.0), Pos2::new(c.x + 3.5, c.y + 5.5));
                let b = Rect::from_min_max(Pos2::new(c.x - 3.0, c.y - 5.5), Pos2::new(c.x + 5.5, c.y + 3.0));
                ui.painter().rect_stroke(a, Rounding::same(2.0), s);
                ui.painter().rect_stroke(b, Rounding::same(2.0), s);
            } else {
                ui.painter().rect_stroke(
                    Rect::from_center_size(c, Vec2::splat(11.0)),
                    Rounding::same(2.5),
                    s,
                );
            }
        }
        WinBtn::Close => {
            ui.painter().line_segment([Pos2::new(c.x - 5.0, c.y - 5.0), Pos2::new(c.x + 5.0, c.y + 5.0)], s);
            ui.painter().line_segment([Pos2::new(c.x + 5.0, c.y - 5.0), Pos2::new(c.x - 5.0, c.y + 5.0)], s);
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// 导航图标：几何点 / 线构图（idx 与 NAV_ITEMS 顺序一致）
fn nav_glyph(p: &egui::Painter, idx: usize, c: Pos2, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    match idx {
        0 => {
            p.circle_stroke(c, 4.4, s);
            for (dx, dy) in [(0.0_f32, -1.0_f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                p.line_segment([Pos2::new(c.x + dx * 5.4, c.y + dy * 5.4), Pos2::new(c.x + dx * 7.4, c.y + dy * 7.4)], s);
            }
        }
        1 => {
            for (i, h) in [3.5_f32, 6.5, 4.5].iter().enumerate() {
                let x = c.x - 5.0 + i as f32 * 5.0;
                p.line_segment([Pos2::new(x, c.y - h * 0.5), Pos2::new(x, c.y + h * 0.5)], s);
            }
        }
        2 => {
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                p.line_segment([Pos2::new(c.x + sx * 7.0, c.y + sy * 2.6), Pos2::new(c.x + sx * 7.0, c.y + sy * 7.0)], s);
                p.line_segment([Pos2::new(c.x + sx * 2.6, c.y + sy * 7.0), Pos2::new(c.x + sx * 7.0, c.y + sy * 7.0)], s);
            }
        }
        3 => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::new(15.0, 9.5)), Rounding::same(4.0), s);
            p.circle_filled(Pos2::new(c.x - 4.0, c.y), 1.6, color);
            p.circle_filled(Pos2::new(c.x + 4.0, c.y), 1.6, color);
        }
        4 => {
            p.rect_stroke(Rect::from_center_size(Pos2::new(c.x, c.y - 0.5), Vec2::new(13.5, 11.5)), Rounding::same(3.0), s);
            p.line_segment([Pos2::new(c.x - 2.6, c.y + 3.6), Pos2::new(c.x + 2.6, c.y + 3.6)], s);
        }
        5 => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::new(14.5, 11.0)), Rounding::same(3.0), s);
            p.line_segment([Pos2::new(c.x - 5.0, c.y + 3.0), Pos2::new(c.x - 1.2, c.y - 1.4)], s);
            p.line_segment([Pos2::new(c.x - 1.2, c.y - 1.4), Pos2::new(c.x + 2.2, c.y + 1.8)], s);
            p.circle_filled(Pos2::new(c.x + 3.4, c.y - 2.6), 1.4, color);
        }
        6 => {
            for i in 0..3 {
                let y = c.y - 4.4 + i as f32 * 4.4;
                p.circle_filled(Pos2::new(c.x - 5.6, y), 1.3, color);
                p.line_segment([Pos2::new(c.x - 2.4, y), Pos2::new(c.x + 6.0, y)], s);
            }
        }
        _ => {
            p.circle_stroke(c, 3.6, s);
            for (dx, dy) in [(0.0_f32, -1.0_f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                p.line_segment([Pos2::new(c.x + dx * 5.2, c.y + dy * 5.2), Pos2::new(c.x + dx * 7.2, c.y + dy * 7.2)], s);
            }
        }
    }
}

fn ads_button_name(lang: Lang, b: AdsButton) -> &'static str {
    match b {
        AdsButton::LeftTrigger => t(lang, "ads_left_trigger"),
        AdsButton::RightTrigger => t(lang, "ads_right_trigger"),
        AdsButton::LeftBumper => t(lang, "ads_left_bumper"),
        AdsButton::RightBumper => t(lang, "ads_right_bumper"),
    }
}

/// Windows 键是否按下（egui 的 Modifiers 没有 Win，录制时直接问系统）
fn win_key_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LWIN, VK_RWIN};
    let down = |vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY| unsafe {
        GetAsyncKeyState(vk.0 as i32) & 0x8000u16 as i16 != 0
    };
    down(VK_LWIN) || down(VK_RWIN)
}

/// egui 按键 → Windows 虚拟键码。
///
/// 只收录 `Hotkey` 的 Display/parse 能**往返一致**的键（这是硬要求：序列化走
/// `Display`，IPC 负载也走 serde→`Display`，一旦写出 `?` 则预设文件反序列化直接失败、
/// 热键丢失）。覆盖：字母 0x41-0x5A、数字 0x30-0x39、F1-F24 0x70-0x87、
/// 方向键 0x25-0x28、Space/Tab/Enter/Esc/Backspace/Delete/Insert/Home/End/PgUp/PgDn，
/// 以及 OEM 符号键（`-` 0xBD、`=` 0xBB、`[` 0xDB、`]` 0xDD、`\` 0xDC、`;` 0xBA、
/// `'` 0xDE、`,` 0xBC、`.` 0xBE、`/` 0xBF、`` ` `` 0xC0）——v1.2.0 起
/// `config::vk_to_key_token` 已为这些 VK 补上同名 token，链路闭合。
///
/// 带 Shift 的符号（`:` `?` `|` `{` `}` `+`）映射到**同一个物理键**的 VK：
/// 修饰键由 egui 的 `Modifiers` 记录成 `Shift`，所以 `Ctrl+Shift+;` 与 `Ctrl+:` 落库一致，
/// 且都能量回得来。纯修饰伪键（Copy/Cut/Paste）没有对应 VK，返回 `None`。
fn key_to_vk(key: egui::Key) -> Option<u32> {
    use egui::Key;
    Some(match key {
        Key::A => 0x41,
        Key::B => 0x42,
        Key::C => 0x43,
        Key::D => 0x44,
        Key::E => 0x45,
        Key::F => 0x46,
        Key::G => 0x47,
        Key::H => 0x48,
        Key::I => 0x49,
        Key::J => 0x4A,
        Key::K => 0x4B,
        Key::L => 0x4C,
        Key::M => 0x4D,
        Key::N => 0x4E,
        Key::O => 0x4F,
        Key::P => 0x50,
        Key::Q => 0x51,
        Key::R => 0x52,
        Key::S => 0x53,
        Key::T => 0x54,
        Key::U => 0x55,
        Key::V => 0x56,
        Key::W => 0x57,
        Key::X => 0x58,
        Key::Y => 0x59,
        Key::Z => 0x5A,
        Key::Num0 => 0x30,
        Key::Num1 => 0x31,
        Key::Num2 => 0x32,
        Key::Num3 => 0x33,
        Key::Num4 => 0x34,
        Key::Num5 => 0x35,
        Key::Num6 => 0x36,
        Key::Num7 => 0x37,
        Key::Num8 => 0x38,
        Key::Num9 => 0x39,
        Key::F1 => 0x70,
        Key::F2 => 0x71,
        Key::F3 => 0x72,
        Key::F4 => 0x73,
        Key::F5 => 0x74,
        Key::F6 => 0x75,
        Key::F7 => 0x76,
        Key::F8 => 0x77,
        Key::F9 => 0x78,
        Key::F10 => 0x79,
        Key::F11 => 0x7A,
        Key::F12 => 0x7B,
        Key::F13 => 0x7C,
        Key::F14 => 0x7D,
        Key::F15 => 0x7E,
        Key::F16 => 0x7F,
        Key::F17 => 0x80,
        Key::F18 => 0x81,
        Key::F19 => 0x82,
        Key::F20 => 0x83,
        Key::F21 => 0x84,
        Key::F22 => 0x85,
        Key::F23 => 0x86,
        Key::F24 => 0x87,
        Key::ArrowUp => 0x26,
        Key::ArrowDown => 0x28,
        Key::ArrowLeft => 0x25,
        Key::ArrowRight => 0x27,
        Key::Space => 0x20,
        Key::Tab => 0x09,
        Key::Enter => 0x0D,
        Key::Escape => 0x1B,
        Key::Backspace => 0x08,
        Key::Delete => 0x2E,
        Key::Insert => 0x2D,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        // ---- OEM 符号键（与 config::vk_to_key_token 的 token 一一对应） ----
        // 同一物理键的不同逻辑字符（Shift 态）映射到同一 VK，修饰键由 Modifiers 记录
        Key::Minus => 0xBD,
        Key::Equals | Key::Plus => 0xBB,
        Key::OpenBracket => 0xDB,
        Key::CloseBracket => 0xDD,
        Key::Backslash | Key::Pipe => 0xDC,
        Key::Semicolon | Key::Colon => 0xBA,
        Key::Quote => 0xDE,
        Key::Comma => 0xBC,
        Key::Period => 0xBE,
        Key::Slash | Key::Questionmark => 0xBF,
        Key::Backtick => 0xC0,
        _ => return None,
    })
}

// ===========================================================================
// 模板
// ===========================================================================

/// 风格模板：借鉴 Crosshair X 的常见预设风格，一键套用参数。
const TEMPLATES: [(&str, fn(&mut Preset)); 8] = [
    ("tpl_apex", |p: &mut Preset| {
        p.shape = CrossShape::HollowCrossDot;
        p.size = 14.0;
        p.thickness = 1.6;
        p.hollow.gap = 3.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 0.85;
        p.hollow.center_dot_size = 2.0;
        p.dynamic.fire_expand_px = 4.0;
        p.dynamic.recover_ms = 140;
    }),
    ("tpl_valorant", |p: &mut Preset| {
        p.shape = CrossShape::Gate;
        p.size = 16.0;
        p.thickness = 1.0;
        p.hollow.gap = 2.0;
        p.color = "#3DFF6E".into();
        p.multicolor = false;
        p.opacity = 0.9;
    }),
    ("tpl_cs2", |p: &mut Preset| {
        p.shape = CrossShape::Cross;
        p.size = 18.0;
        p.thickness = 1.5;
        p.hollow.gap = 0.0;
        p.color = "#00FF00".into();
        p.multicolor = false;
        p.opacity = 0.9;
    }),
    ("tpl_sniper", |p: &mut Preset| {
        p.shape = CrossShape::RingDot;
        p.size = 26.0;
        p.thickness = 2.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 0.7;
        p.hollow.center_dot_size = 2.5;
    }),
    ("tpl_classic", |p: &mut Preset| {
        p.shape = CrossShape::Cross;
        p.size = 20.0;
        p.thickness = 2.0;
        p.hollow.gap = 0.0;
        p.color = "#FF0000".into();
        p.multicolor = false;
        p.opacity = 0.8;
    }),
    ("tpl_thick_gate", |p: &mut Preset| {
        p.shape = CrossShape::Gate;
        p.size = 24.0;
        p.thickness = 4.0;
        p.hollow.gap = 3.0;
        p.color = "#FFFFFF".into();
        p.multicolor = false;
        p.opacity = 1.0;
    }),
    ("tpl_dot", |p: &mut Preset| {
        p.shape = CrossShape::Dot;
        p.size = 6.0;
        p.color = "#FF2D2D".into();
        p.opacity = 0.9;
    }),
    ("tpl_cross_x", |p: &mut Preset| {
        p.shape = CrossShape::XShape;
        p.size = 18.0;
        p.thickness = 2.0;
        p.color = "#FFA500".into();
        p.multicolor = false;
        p.opacity = 0.85;
    }),
];

/// 模板缩略图基底：清掉当前预设里会干扰缩略图表现的字段
/// （模板只负责「准星造型」，不该把用户当前的多色 / 描边 / 图片带进缩略图）
fn template_base(p: &Preset) -> Preset {
    let mut b = p.clone();
    b.multicolor = false;
    b.outline.enabled = false;
    b.hollow.gap = 0.0;
    b.hollow.center_dot_size = 0.0;
    b.rotation = 0.0;
    b.opacity = 1.0;
    b.dynamic.fire_expand_px = 0.0;
    b.dynamic.recoil_indicator = false;
    b.image.path.clear();
    b
}

// ===========================================================================
// eframe::App
// ===========================================================================

impl eframe::App for AcajaApp {
    /// 透明窗口：清屏色必须是全透明，系统亚克力背板才能透出来
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // 首帧起每帧调用：窗口就绪后一次性加系统圆角 + 亚克力（失败静默降级）
        crate::system::windowfx::apply_once("ACAJA");
        let dark = match self.theme.as_str() {
            "light" => false,
            "dark" => true,
            // 「自动」跟随系统（eframe 默认把系统主题写进 egui visuals）
            _ => ctx.style().visuals.dark_mode,
        };
        self.pal = if dark { Palette::dark() } else { Palette::light() };
        let pal = self.pal;
        self.poll_hotkey_recording(ctx);

        egui::CentralPanel::default()
            .frame(Frame::none())
            .show(ctx, |ui| {
                apply_style(ui, &pal);
                let full = ui.max_rect();
                paint_backdrop(ui.painter(), full, &pal);

                let title_h = 54.0;
                let bottom_h = 58.0;
                let title_rect = Rect::from_min_size(Pos2::new(full.left() + PAD, full.top() + 8.0), Vec2::new(full.width() - PAD * 2.0, title_h));
                self.title_bar(ui, &pal, title_rect);
                let bottom_rect = Rect::from_min_size(
                    Pos2::new(full.left() + PAD, full.bottom() - bottom_h - 10.0),
                    Vec2::new(full.width() - PAD * 2.0, bottom_h),
                );

                let body_top = title_rect.bottom() + 6.0;
                let body_bottom = bottom_rect.top() - 8.0;
                let nav_rect = Rect::from_min_size(Pos2::new(full.left() + PAD, body_top), Vec2::new(NAV_W, body_bottom - body_top));
                let content_rect = Rect::from_min_size(
                    Pos2::new(nav_rect.right() + 16.0, body_top),
                    Vec2::new(full.right() - PAD - 16.0 - nav_rect.right(), body_bottom - body_top),
                );

                // ---- 导航（玻璃面板） ----
                let mut nav_shapes = Vec::new();
                push_panel(&mut nav_shapes, nav_rect, R_CARD, &pal, 0.0);
                ui.painter().add(Shape::Vec(nav_shapes));
                let mut nav_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(nav_rect.shrink2(Vec2::new(8.0, 10.0)))
                        .layout(Layout::top_down(Align::Min)),
                );
                nav_ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);
                self.nav_ui(&mut nav_ui, &pal);

                // ---- 内容（滚动） ----
                let mut content_ui = ui.new_child(
                    UiBuilder::new().max_rect(content_rect).layout(Layout::top_down(Align::Min)),
                );
                content_ui.set_clip_rect(content_rect);
                ScrollArea::vertical().auto_shrink([false, false]).show(&mut content_ui, |ui| {
                    apply_style(ui, &pal);
                    match self.active_section {
                        SEC_STYLE => self.section_style(ui, &pal),
                        SEC_DYNAMIC => self.section_dynamic(ui, &pal),
                        SEC_POSITION => self.section_position(ui, &pal),
                        SEC_GAMEPAD => self.section_gamepad(ui, &pal),
                        SEC_HOTKEY => self.section_hotkey(ui, &pal),
                        SEC_IMAGE => self.section_image(ui, &pal),
                        SEC_PRESETS => self.section_presets(ui, &pal),
                        _ => self.section_system(ui, &pal),
                    }
                    ui.add_space(4.0);
                });

                // ---- 底部操作条 ----
                self.bottom_bar(ui, &pal, bottom_rect);

                // ---- 无边框窗口的自绘缩放边 ----
                resize_grips(ui, full);
            });

        // ---- 帧末：改动即时推给主程序（真实准星跟随；写盘仍由「应用」/关窗负责） ----
        if self.dirty {
            self.live_push();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // 退出前自动保存当前准星设置 → 下次打开沿用（不需要手动点保存）
        let (name, preset) = (self.active_name.clone(), self.preset.clone());
        let result = {
            let mut store = self.store.lock();
            let res = store.save_preset(&name, &preset);
            if res.is_ok() {
                let _ = store.save_app();
            }
            res
        };
        match result {
            Ok(()) => info!("退出前已自动保存预设 {name}"),
            Err(e) => warn!("退出前自动保存失败: {e}"),
        }
    }
}

/// 无边框窗口的 8 向缩放热区（系统装饰被关掉了，边缘得自己接管）
fn resize_grips(ui: &mut Ui, full: Rect) {
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    if maximized {
        return;
    }
    let g = 5.0;
    let grip = |rect: Rect, dir: ResizeDirection, cursor: CursorIcon| {
        let resp = ui.interact(rect, Id::new(("grip", dir as i32)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    };
    grip(
        Rect::from_min_max(full.min, Pos2::new(full.right(), full.top() + g)),
        ResizeDirection::North,
        CursorIcon::ResizeNorth,
    );
    grip(
        Rect::from_min_max(Pos2::new(full.left(), full.bottom() - g), full.max),
        ResizeDirection::South,
        CursorIcon::ResizeSouth,
    );
    grip(
        Rect::from_min_max(full.min, Pos2::new(full.left() + g, full.bottom())),
        ResizeDirection::West,
        CursorIcon::ResizeWest,
    );
    grip(
        Rect::from_min_max(Pos2::new(full.right() - g, full.top()), full.max),
        ResizeDirection::East,
        CursorIcon::ResizeEast,
    );
    grip(
        Rect::from_min_max(full.min, Pos2::new(full.left() + g, full.top() + g)),
        ResizeDirection::NorthWest,
        CursorIcon::ResizeNorthWest,
    );
    grip(
        Rect::from_min_max(Pos2::new(full.right() - g, full.top()), Pos2::new(full.right(), full.top() + g)),
        ResizeDirection::NorthEast,
        CursorIcon::ResizeNorthEast,
    );
    grip(
        Rect::from_min_max(Pos2::new(full.left(), full.bottom() - g), Pos2::new(full.left() + g, full.bottom())),
        ResizeDirection::SouthWest,
        CursorIcon::ResizeSouthWest,
    );
    grip(
        Rect::from_min_max(Pos2::new(full.right() - g, full.bottom() - g), full.max),
        ResizeDirection::SouthEast,
        CursorIcon::ResizeSouthEast,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 录制的键必须能经 `Hotkey` 的 Display → parse 往返还原。
    ///
    /// 这不是形式主义：`Hotkey` 的序列化就是 `Display`（预设文件与 IPC 负载都走它），
    /// 一旦某个 VK 被渲染成 `?`，整条热键在读回时解析失败 → 预设反序列化报错。
    /// 所以断言里显式检查显示字符串不含 `?`，并用 `Key::ALL` 做穷举而不是抽样。
    #[test]
    fn recorded_keys_round_trip_through_hotkey() {
        let mut covered = 0;
        for key in egui::Key::ALL {
            let Some(vk) = key_to_vk(*key) else { continue };
            covered += 1;
            for mods in [0u32, MOD_CONTROL, MOD_CONTROL | MOD_SHIFT, MOD_ALT | MOD_WIN] {
                let hk = Hotkey { modifiers: mods, vk };
                let text = hk.to_string();
                assert!(!text.contains('?'), "{key:?} 显示为「{text}」含未映射的 '?'");
                assert_eq!(Hotkey::parse(&text), Some(hk), "{key:?} → 「{text}」不能往返还原");
            }
        }
        assert!(covered >= 70, "可录制的按键覆盖太少: {covered}");

        // 抽查具体 VK：字母 / 数字 / F 键 / 方向键 / OEM 符号
        for (key, vk) in [
            (egui::Key::A, 0x41),
            (egui::Key::Num5, 0x35),
            (egui::Key::F12, 0x7B),
            (egui::Key::F24, 0x87),
            (egui::Key::ArrowUp, 0x26),
            (egui::Key::Minus, 0xBD),
            (egui::Key::Equals, 0xBB),
            (egui::Key::OpenBracket, 0xDB),
            (egui::Key::Backslash, 0xDC),
            (egui::Key::Semicolon, 0xBA),
            (egui::Key::Quote, 0xDE),
            (egui::Key::Slash, 0xBF),
            (egui::Key::Backtick, 0xC0),
        ] {
            assert_eq!(key_to_vk(key), Some(vk), "{key:?} 的 VK 映射不对");
        }
        // 纯修饰伪键没有对应 VK，必须明确拒绝
        assert_eq!(key_to_vk(egui::Key::Copy), None);
        assert_eq!(key_to_vk(egui::Key::Cut), None);
        assert_eq!(key_to_vk(egui::Key::Paste), None);
        // 空热键
        assert!(Hotkey::default().is_empty());
    }
}
