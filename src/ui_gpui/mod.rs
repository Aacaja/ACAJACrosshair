//! ACAJA v1.3 设置界面重构预览（GPUI Kit）。
//!
//! 目标不是复刻旧 egui 的每一层，而是验证三件事：
//! 1. GPUI 的 Windows 后端能拿到亚克力 / Mica（`Window::set_background_appearance`）；
//! 2. 自绘预览、CJK 字体、圆角卡片和动效在 GPUI 下能达到更干净的观感；
//! 3. 现有 `Preset` / `PresetStore` / `WM_COPYDATA` 可以原样复用。
//!
//! 设计取舍：窗口级真模糊 + 高不透明度的深色表面（约 90%），
//! 不再追求旧版「整窗 35% 透明」的极端透感——优先可读性和高级感。
//! 面板内部的“玻璃”用半透明填充、1px 镜面边和分层色块伪造；
//! GPUI 目前没有逐元素 backdrop blur（gpui-kit#3598）。

use std::borrow::Cow;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Theme, ThemeMode, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Entity, FontWeight, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, PathBuilder, Render, Rgba, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, canvas, div, point, px,
    rgba as gpui_rgba, size,
};

use crate::config::{GlassMode, Preset, PresetStore, QuadColors, Shape as CrossShape};
use crate::i18n::Lang;
use crate::overlay::parse_hex;
use crate::overlay::shapes::{self, Prim, ShapeParams, SLOT_BOTTOM, SLOT_LEFT, SLOT_RIGHT, SLOT_TOP};
use crate::ui::strings::{shape_name, t};

// ===========================================================================
// 设计令牌（v1.3 深空玻璃，但表面更实、更克制）
// ===========================================================================

const FONT_SANS: &str = "Noto Sans SC Thin";
const FONT_SERIF: &str = "Noto Serif SC ExtraLight";

const BG: u32 = 0x08080ADB;
const TITLE_BAR: u32 = 0x0B0B0FE8;
const SIDEBAR: u32 = 0x0B0B0FD8;
const CARD: u32 = 0x14141AEE;
const CARD_HOVER: u32 = 0x1B1B24FF;
const CARD_ACTIVE: u32 = 0x22222CFF;
const CONTROL: u32 = 0x101015FF;
const BORDER: u32 = 0x2A2A36CC;
const BORDER_HI: u32 = 0x3C3C4ECC;
const TEXT: u32 = 0xF4F5F8FF;
const MUTED: u32 = 0xA7ABB8FF;
const DIM: u32 = 0x71747FFF;
const ACCENT: u32 = 0x7C6CF0FF;
const ACCENT_2: u32 = 0x4F7DFFFF;
const ACCENT_SOFT: u32 = 0x2A2344FF;
const SUCCESS: u32 = 0x4ADE80FF;
const WARN: u32 = 0xF5A524FF;
const DANGER: u32 = 0xFF5C6CFF;

fn c(value: u32) -> Hsla {
    Hsla::from(gpui_rgba(value))
}

fn ca(value: u32, alpha: f32) -> Hsla {
    Hsla::from(Rgba { a: alpha, ..gpui_rgba(value) })
}

fn hex_color(hex: &str, alpha: f32) -> Hsla {
    let (r, g, b) = parse_hex(hex);
    Hsla::from(Rgba { r, g, b, a: alpha.clamp(0.0, 1.0) })
}

// ===========================================================================
// 分区
// ===========================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Style,
    Dynamic,
    Position,
    Gamepad,
    Hotkey,
    Image,
    Presets,
    System,
}

impl Section {
    const ALL: [Section; 8] = [
        Section::Style,
        Section::Dynamic,
        Section::Position,
        Section::Gamepad,
        Section::Hotkey,
        Section::Image,
        Section::Presets,
        Section::System,
    ];

    fn nav_key(self) -> &'static str {
        match self {
            Section::Style => "nav_style",
            Section::Dynamic => "nav_dynamic",
            Section::Position => "nav_position",
            Section::Gamepad => "nav_gamepad",
            Section::Hotkey => "nav_hotkey",
            Section::Image => "nav_image",
            Section::Presets => "nav_presets",
            Section::System => "nav_system",
        }
    }

    fn title_key(self) -> &'static str {
        match self {
            Section::Style => "shape_style",
            Section::Dynamic => "dynamic",
            Section::Position => "position",
            Section::Gamepad => "gamepad",
            Section::Hotkey => "hotkey",
            Section::Image => "nav_image",
            Section::Presets => "presets",
            Section::System => "system",
        }
    }
}

/// GPUI 预览里可切换的窗口材质。
#[derive(Clone, Copy, PartialEq, Eq)]
enum GpuiGlass {
    Acrylic,
    Mica,
    Transparent,
    Opaque,
}

impl GpuiGlass {
    const ALL: [GpuiGlass; 4] = [
        GpuiGlass::Acrylic,
        GpuiGlass::Mica,
        GpuiGlass::Transparent,
        GpuiGlass::Opaque,
    ];

    fn appearance(self) -> WindowBackgroundAppearance {
        match self {
            GpuiGlass::Acrylic => WindowBackgroundAppearance::Blurred,
            GpuiGlass::Mica => WindowBackgroundAppearance::MicaBackdrop,
            GpuiGlass::Transparent => WindowBackgroundAppearance::Transparent,
            GpuiGlass::Opaque => WindowBackgroundAppearance::Opaque,
        }
    }

    fn label(self) -> &'static str {
        match self {
            GpuiGlass::Acrylic => "Acrylic",
            GpuiGlass::Mica => "Mica",
            GpuiGlass::Transparent => "Transparent",
            GpuiGlass::Opaque => "Opaque",
        }
    }

    fn note(self) -> &'static str {
        match self {
            GpuiGlass::Acrylic => "Win10 1809+ / Win11",
            GpuiGlass::Mica => "Win11 22H2+",
            GpuiGlass::Transparent => "no blur, alpha only",
            GpuiGlass::Opaque => "safe fallback",
        }
    }

    fn from_mode(mode: GlassMode) -> Self {
        match mode {
            GlassMode::Acrylic | GlassMode::Auto => GpuiGlass::Acrylic,
            GlassMode::Backdrop => GpuiGlass::Mica,
            GlassMode::Off => GpuiGlass::Opaque,
        }
    }
}

// ===========================================================================
// 入口
// ===========================================================================

pub fn run(store: Arc<Mutex<PresetStore>>, glass_test: bool) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            install_fonts(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            configure_theme(cx);

            let store_for_view = store.clone();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1240.), px(800.)), cx)),
                window_min_size: Some(size(px(1080.), px(700.))),
                window_background: WindowBackgroundAppearance::Blurred,
                app_id: Some("acaja-studio".into()),
                ..TitleBar::window_options()
            };

            let result = gpui_kit::open_window(options, cx, move |window, cx| {
                window.activate_window();
                window.set_window_title("ACAJA Studio");
                Theme::change(ThemeMode::Dark, Some(window), cx);
                configure_theme(cx);
                cx.new(|cx| SettingsApp::new(window, cx, store_for_view, glass_test))
            });

            if let Err(err) = result {
                log::error!("创建 GPUI 窗口失败: {err}");
                return;
            }
            cx.activate(true);
        });
}

fn install_fonts(cx: &mut App) {
    let fonts = vec![
        Cow::Borrowed(include_bytes!("../../assets/fonts/ACAJASans-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../assets/fonts/ACAJASans-Bold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../../assets/fonts/ACAJASerif-SemiBold.ttf").as_slice()),
    ];
    if let Err(err) = cx.text_system().add_fonts(fonts) {
        log::warn!("内置字体加载失败: {err}");
    }
    let names = cx.text_system().all_font_names();
    log::info!("GPUI 可用字体: {:?}", names);
}

fn configure_theme(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.primary = c(ACCENT);
        theme.primary_foreground = c(0xFFFFFFFF);
        theme.background = c(BG);
        theme.foreground = c(TEXT);
        theme.border = c(BORDER);
        theme.muted = c(CONTROL);
        theme.muted_foreground = c(MUTED);
        theme.secondary = c(CARD);
        theme.secondary_hover = c(CARD_HOVER);
        theme.secondary_active = c(CARD_ACTIVE);
        theme.danger = c(DANGER);
        theme.danger_foreground = c(0xFFFFFFFF);
        theme.danger_active = c(0xE04455FF);
        theme.title_bar = c(TITLE_BAR);
        theme.title_bar_border = c(BORDER);
        theme.radius = px(12.);
        theme.radius_lg = px(20.);
        theme.font_family = FONT_SANS.into();
        theme.font_size = px(13.);
    });
}

// ===========================================================================
// 状态
// ===========================================================================

struct SettingsApp {
    store: Arc<Mutex<PresetStore>>,
    draft: Preset,
    preset_name: String,
    section: Section,
    lang: Lang,
    status: SharedString,
    glass: GpuiGlass,
    last_push: Instant,
    /// 关闭窗口时是否保存（「关闭」按钮会先置 false，标题栏 X 保持 true）
    save_on_drop: bool,

    size_slider: Entity<SliderState>,
    thickness_slider: Entity<SliderState>,
    opacity_slider: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsApp {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        store: Arc<Mutex<PresetStore>>,
        glass_test: bool,
    ) -> Self {
        let (draft, preset_name, stored_mode) = {
            let store = store.lock();
            (store.get_active().clone(), store.active_name(), store.app.glass.mode)
        };

        let glass = if glass_test {
            GpuiGlass::Acrylic
        } else {
            GpuiGlass::from_mode(stored_mode)
        };
        window.set_background_appearance(glass.appearance());
        log::info!("窗口材质: {}", glass.label());

        let size_slider = cx.new(|_| {
            SliderState::new()
                .min(2.0)
                .max(80.0)
                .step(0.5)
                .default_value(draft.size)
        });
        let thickness_slider = cx.new(|_| {
            SliderState::new()
                .min(0.5)
                .max(12.0)
                .step(0.1)
                .default_value(draft.thickness)
        });
        let opacity_slider = cx.new(|_| {
            SliderState::new()
                .min(0.1)
                .max(1.0)
                .step(0.01)
                .default_value(draft.opacity)
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &size_slider,
            window,
            |this, _state, event: &SliderEvent, _window, cx| {
                if let SliderEvent::Change(value) = event {
                    this.draft.size = value.start().clamp(2.0, 80.0);
                    this.mark_dirty(cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &thickness_slider,
            window,
            |this, _state, event: &SliderEvent, _window, cx| {
                if let SliderEvent::Change(value) = event {
                    this.draft.thickness = value.start().clamp(0.5, 12.0);
                    this.mark_dirty(cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &opacity_slider,
            window,
            |this, _state, event: &SliderEvent, _window, cx| {
                if let SliderEvent::Change(value) = event {
                    this.draft.opacity = value.start().clamp(0.1, 1.0);
                    this.mark_dirty(cx);
                }
            },
        ));

        let connected = crate::ipc::find_backend().is_some();
        let status = if connected {
            t(Lang::Zh, "backend_connected")
        } else {
            t(Lang::Zh, "backend_file_mode")
        };

        Self {
            store,
            draft,
            preset_name,
            section: Section::Style,
            lang: Lang::Zh,
            status: status.into(),
            glass,
            last_push: Instant::now() - Duration::from_secs(1),
            save_on_drop: true,
            size_slider,
            thickness_slider,
            opacity_slider,
            _subscriptions: subscriptions,
        }
    }

    fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        self.save_on_drop = true;
        self.status = t(self.lang, "backend_dirty").into();
        self.live_push(cx);
    }

    fn live_push(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if now.duration_since(self.last_push) < Duration::from_millis(120) {
            cx.notify();
            return;
        }
        self.last_push = now;
        self.push_to_backend(false, cx);
    }

    fn push_to_backend(&mut self, save: bool, cx: &mut Context<Self>) {
        let mut message: Option<String> = None;

        if save {
            let mut store = self.store.lock();
            match store.save_preset(&self.preset_name, &self.draft) {
                Ok(()) => {
                    let _ = store.save_app();
                }
                Err(err) => {
                    log::warn!("保存预设失败: {err}");
                    message = Some(format!("{}: {err}", t(self.lang, "error")));
                }
            }
        }

        if message.is_none() {
            if let Some(hwnd) = crate::ipc::find_backend() {
                let payload = crate::ipc::preset_payload(&self.draft, true);
                let ok = crate::ipc::send_json_timeout(
                    hwnd,
                    crate::ipc::IPC_TAG_PRESET,
                    &payload,
                    120,
                );
                message = Some(if ok {
                    t(self.lang, "pushed_ok").to_string()
                } else {
                    t(self.lang, "backend_dirty").to_string()
                });
            } else {
                message = Some(t(self.lang, "pushed_via_file").to_string());
            }
        }

        self.status = message.unwrap_or_default().into();
        cx.notify();
    }

    fn set_lang(&mut self, lang: Lang, cx: &mut Context<Self>) {
        self.lang = lang;
        let connected = crate::ipc::find_backend().is_some();
        self.status = if connected {
            t(lang, "backend_connected")
        } else {
            t(lang, "backend_file_mode")
        }
        .into();
        cx.notify();
    }

    fn set_multicolor(this: &mut Self, value: bool) {
        // 预览版没有四象限编辑器：如果当前四色是统一色，先同步成主色，
        // 避免用户切到多色模式时突然变回默认红色。真正的四象限编辑后续补。
        if value && this.draft.colors.is_uniform() {
            let color = this.draft.color.clone();
            this.draft.colors = QuadColors::from_single(&color);
        }
        this.draft.multicolor = value;
    }

    fn set_outline(this: &mut Self, value: bool) {
        this.draft.outline.enabled = value;
    }
}

impl Drop for SettingsApp {
    fn drop(&mut self) {
        // 与旧设置进程一致：标题栏 X 关窗即保存工作副本；
        // 底部「关闭」按钮会先置 save_on_drop=false，实现真正的“不保存退出”。
        if !self.save_on_drop {
            return;
        }
        let mut store = self.store.lock();
        let _ = store.save_preset(&self.preset_name, &self.draft);
        let _ = store.save_app();
    }
}

// ===========================================================================
// 渲染
// ===========================================================================

impl Render for SettingsApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(c(BG))
            .text_color(c(TEXT))
            .font_family(FONT_SANS)
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(self.render_sidebar(cx))
                    .child(self.render_main(cx)),
            )
            .child(self.render_bottom_bar(cx))
    }
}

impl SettingsApp {
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = crate::ipc::find_backend().is_some();
        let lang_label = match self.lang {
            Lang::Zh => "中 / EN",
            Lang::En => "EN / 中",
        };

        TitleBar::new()
            .bg(c(TITLE_BAR))
            .border_color(c(BORDER))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(2.))
                    .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(c(ACCENT)))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .child("ACAJA"),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(c(DIM))
                            .child("STUDIO"),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .mr(px(2.))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(6.))
                            .h(px(22.))
                            .px(px(9.))
                            .rounded_full()
                            .bg(c(CONTROL))
                            .border_1()
                            .border_color(c(BORDER))
                            .child(
                                div()
                                    .w(px(6.))
                                    .h(px(6.))
                                    .rounded_full()
                                    .bg(if connected { c(SUCCESS) } else { c(WARN) }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(c(MUTED))
                                    .child(if connected { "LIVE" } else { "FILE" }),
                            ),
                    )
                    .child(
                        div()
                            .id("lang-toggle")
                            .h(px(22.))
                            .px(px(9.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .hover(|style| style.bg(c(CARD_HOVER)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let next = match this.lang {
                                    Lang::Zh => Lang::En,
                                    Lang::En => Lang::Zh,
                                };
                                this.set_lang(next, cx);
                            }))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(c(MUTED))
                                    .child(lang_label),
                            ),
                    ),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = v_flex().gap(px(3.));
        for section in Section::ALL {
            nav = nav.child(self.nav_item(section, cx));
        }

        v_flex()
            .w(px(228.))
            .h_full()
            .p(px(14.))
            .gap(px(3.))
            .bg(c(SIDEBAR))
            .border_r_1()
            .border_color(c(BORDER))
            .child(self.sidebar_brand())
            .child(div().h(px(12.)))
            .child(nav)
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap(px(3.))
                    .p(px(11.))
                    .rounded(px(12.))
                    .bg(c(CONTROL))
                    .border_1()
                    .border_color(c(BORDER))
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(c(ACCENT))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("GPUI PREVIEW"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(c(DIM))
                            .child(format!("v{} · rebuild", crate::VERSION)),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(c(DIM))
                            .child("D2D overlay unchanged"),
                    ),
            )
    }

    fn sidebar_brand(&self) -> impl IntoElement {
        h_flex()
            .items_center()
            .gap(px(10.))
            .px(px(8.))
            .py(px(10.))
            .child(
                div()
                    .w(px(38.))
                    .h(px(38.))
                    .rounded(px(12.))
                    .bg(c(ACCENT_SOFT))
                    .border_1()
                    .border_color(ca(ACCENT, 0.35))
                    .child(
                        canvas(
                            |bounds, _, _| bounds,
                            |_, bounds, window, _| {
                                let cx = f32::from(bounds.center().x);
                                let cy = f32::from(bounds.center().y);
                                paint_cross_mark(window, cx, cy, 9.0, c(0xFFFFFFFF));
                            },
                        )
                        .size_full(),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_family(FONT_SERIF)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .child("ACAJA"),
                    )
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(c(DIM))
                            .child("CROSSHAIR STUDIO"),
                    ),
            )
    }

    fn nav_item(&self, section: Section, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.section == section;
        let label = t(self.lang, section.nav_key());

        let mut item = div()
            .id(("nav", section as usize))
            .h(px(42.))
            .px(px(10.))
            .rounded(px(11.))
            .flex()
            .items_center()
            .gap(px(10.))
            .cursor_pointer()
            .when(active, |this| {
                this.bg(c(ACCENT_SOFT))
                    .border_1()
                    .border_color(ca(ACCENT, 0.38))
            })
            .when(!active, |this| this.hover(|style| style.bg(c(CARD_HOVER))))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.section = section;
                cx.notify();
            }))
            .child(nav_glyph(section, active))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.5))
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if active { c(TEXT) } else { c(MUTED) })
                    .child(label),
            );

        if active {
            item = item.child(div().w(px(3.)).h(px(14.)).rounded_full().bg(c(ACCENT)));
        }
        item
    }

    fn render_main(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let section: gpui_kit::AnyElement = match self.section {
            Section::Style => self.render_style(cx),
            Section::System => self.render_system(cx),
            _ => self.render_placeholder(cx),
        };

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .child(
                div()
                    .id("main-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .p(px(28.))
                            .gap(px(20.))
                            .child(self.render_header(cx))
                            .child(section),
                    ),
            )
    }

    fn render_header(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let title = t(self.lang, self.section.title_key());
        let subtitle = match self.section {
            Section::Style => match self.lang {
                Lang::Zh => "为准星调校每一像素，改动实时同步到屏幕。",
                Lang::En => "Tune every pixel — changes reach the screen live.",
            },
            Section::System => match self.lang {
                Lang::Zh => "窗口材质与平台行为。Win10 用亚克力，Win11 22H2+ 可试 Mica。",
                Lang::En => "Window material and platform behavior. Acrylic on Win10; Mica on Win11 22H2+.",
            },
            _ => match self.lang {
                Lang::Zh => "这个分区正在用 GPUI 重做，预览版先聚焦「形状样式」和「系统」。",
                Lang::En => "This section is being rebuilt in GPUI. The preview focuses on Style and System.",
            },
        };

        h_flex()
            .items_end()
            .justify_between()
            .gap(px(20.))
            .child(
                v_flex()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_family(FONT_SERIF)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(MUTED))
                            .child(subtitle),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(7.))
                    .h(px(30.))
                    .px(px(12.))
                    .rounded_full()
                    .bg(c(CARD))
                    .border_1()
                    .border_color(c(BORDER))
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(c(ACCENT_2)))
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(c(DIM))
                            .child("PRESET"),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .child(self.preset_name.clone()),
                    ),
            )
    }

    fn render_style(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        v_flex()
            .gap(px(20.))
            .child(
                h_flex()
                    .gap(px(20.))
                    .items_stretch()
                    .child(self.render_preview_card(cx))
                    .child(self.render_quick_controls(cx)),
            )
            .child(self.render_shape_gallery(cx))
            .into_any_element()
    }

    fn render_preview_card(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let shape_label = shape_name(self.lang, self.draft.shape);
        let size_label = format!("{:.0} px", self.draft.size);
        let thickness_label = format!("{:.1} px", self.draft.thickness);
        let opacity_label = format!("{:.0}%", self.draft.opacity * 100.0);
        let preview = self.draft.clone();

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h(px(432.))
            .rounded(px(24.))
            .overflow_hidden()
            .bg(c(0x0A0A0FFF))
            .border_1()
            .border_color(c(BORDER))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        canvas(
                            move |bounds, _, _| bounds,
                            move |_, bounds, window, _| {
                                paint_preview(window, bounds, &preview);
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(top_chip("LIVE PREVIEW", ACCENT))
                    .child(
                        div()
                            .absolute()
                            .top(px(14.))
                            .right(px(14.))
                            .h(px(22.))
                            .px(px(9.))
                            .rounded_full()
                            .bg(ca(0x000000, 0.45))
                            .border_1()
                            .border_color(ca(0xFFFFFF, 0.10))
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(9.5))
                                    .text_color(c(MUTED))
                                    .child("CENTER · 1:1"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .h(px(54.))
                    .px(px(18.))
                    .items_center()
                    .justify_between()
                    .bg(c(0x0D0D12FF))
                    .border_t_1()
                    .border_color(c(BORDER))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(c(TEXT))
                                    .child(shape_label),
                            )
                            .child(div().w(px(1.)).h(px(12.)).bg(c(BORDER)))
                            .child(div().text_size(px(10.5)).text_color(c(MUTED)).child(size_label))
                            .child(div().text_size(px(10.5)).text_color(c(DIM)).child("·"))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(c(MUTED))
                                    .child(thickness_label),
                            )
                            .child(div().text_size(px(10.5)).text_color(c(DIM)).child("·"))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(c(MUTED))
                                    .child(opacity_label),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(c(SUCCESS)))
                            .child(div().text_size(px(9.5)).text_color(c(DIM)).child("LIVE")),
                    ),
            )
    }

    fn render_quick_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = ["#FF3B30", "#FFD60A", "#30D158", "#64D2FF", "#BF5AF2", "#FFFFFF"];
        let mut swatches = h_flex().gap(px(8.)).items_center();
        for (index, hex) in colors.iter().enumerate() {
            let selected = self.draft.color.eq_ignore_ascii_case(hex);
            let value = *hex;
            swatches = swatches.child(
                div()
                    .id(("swatch", index))
                    .w(px(26.))
                    .h(px(26.))
                    .rounded_full()
                    .bg(hex_color(value, 1.0))
                    .border_2()
                    .border_color(if selected { c(TEXT) } else { ca(0xFFFFFF, 0.10) })
                    .cursor_pointer()
                    .hover(|style| style.border_color(ca(0xFFFFFF, 0.40)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.draft.color = value.to_string();
                        if !this.draft.multicolor {
                            this.draft.colors = QuadColors::from_single(value);
                        }
                        this.mark_dirty(cx);
                    })),
            );
        }

        v_flex()
            .w(px(392.))
            .flex_shrink_0()
            .gap(px(16.))
            .p(px(22.))
            .rounded(px(24.))
            .bg(c(CARD))
            .border_1()
            .border_color(c(BORDER))
            .child(section_label("形状与样式"))
            .child(
                v_flex()
                    .gap(px(9.))
                    .child(row_label("主色", "Accent"))
                    .child(swatches)
                    .when(self.draft.multicolor, |this| {
                        this.child(
                            div()
                                .text_size(px(9.5))
                                .text_color(c(DIM))
                                .child(match self.lang {
                                    Lang::Zh => "多色模式下由四象限配色决定；四象限编辑将在后续提交补齐。",
                                    Lang::En => "Multicolor uses quadrant colors; quadrant editing lands in a follow-up.",
                                }),
                        )
                    }),
            )
            .child(self.slider_row(t(self.lang, "size"), self.draft.size, "px", &self.size_slider))
            .child(self.slider_row(
                t(self.lang, "thickness"),
                self.draft.thickness,
                "px",
                &self.thickness_slider,
            ))
            .child(self.slider_row(
                t(self.lang, "opacity"),
                self.draft.opacity * 100.0,
                "%",
                &self.opacity_slider,
            ))
            .child(div().h(px(1.)).bg(c(BORDER)))
            .child(self.switch_row(
                "多色模式",
                self.draft.multicolor,
                cx,
                Self::set_multicolor,
            ))
            .child(self.switch_row("描边", self.draft.outline.enabled, cx, Self::set_outline))
    }

    fn slider_row(
        &self,
        label: &'static str,
        value: f32,
        unit: &'static str,
        slider: &Entity<SliderState>,
    ) -> impl IntoElement {
        v_flex()
            .gap(px(7.))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(11.5)).text_color(c(MUTED)).child(label))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .child(format!("{value:.1} {unit}")),
                    ),
            )
            .child(Slider::new(slider).w_full())
    }

    fn switch_row(
        &self,
        label: &'static str,
        checked: bool,
        cx: &mut Context<Self>,
        setter: fn(&mut Self, bool),
    ) -> impl IntoElement {
        h_flex()
            .items_center()
            .justify_between()
            .h(px(30.))
            .child(div().text_size(px(11.5)).text_color(c(MUTED)).child(label))
            .child(
                Switch::new(label)
                    .checked(checked)
                    .on_change(cx.listener(move |this, value, _, cx| {
                        setter(this, *value);
                        this.mark_dirty(cx);
                    })),
            )
    }

    fn render_shape_gallery(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = v_flex().gap(px(8.));
        for chunk in CrossShape::ALL.chunks(10) {
            let mut row = h_flex().gap(px(8.));
            for shape in chunk {
                row = row.child(self.shape_tile(*shape, cx));
            }
            rows = rows.child(row);
        }

        v_flex()
            .gap(px(10.))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(section_label(t(self.lang, "shape")))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(c(DIM))
                            .child(format!("{} SHAPES", CrossShape::ALL.len())),
                    ),
            )
            .child(rows)
    }

    fn shape_tile(&self, shape: CrossShape, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.draft.shape == shape;
        let color = if selected {
            hex_color(&self.draft.color, 1.0)
        } else {
            c(MUTED)
        };
        let label = shape_name(self.lang, shape);

        div()
            .id(("shape", shape as usize))
            .w(px(84.))
            .h(px(84.))
            .rounded(px(14.))
            .border_1()
            .border_color(if selected { c(ACCENT) } else { c(BORDER) })
            .bg(if selected { c(ACCENT_SOFT) } else { c(0x121218FF) })
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .cursor_pointer()
            .hover(|style| style.bg(c(CARD_HOVER)).border_color(c(BORDER_HI)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.draft.shape = shape;
                this.mark_dirty(cx);
            }))
            .child(shape_thumb(shape, color))
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(if selected { c(TEXT) } else { c(DIM) })
                    .child(label),
            )
    }

    fn render_system(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut options = h_flex().gap(px(10.));
        for choice in GpuiGlass::ALL {
            let selected = self.glass == choice;
            options = options.child(
                div()
                    .id(("glass", choice as usize))
                    .flex_1()
                    .p(px(14.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(if selected { c(ACCENT) } else { c(BORDER) })
                    .bg(if selected { c(ACCENT_SOFT) } else { c(0x121218FF) })
                    .cursor_pointer()
                    .hover(|style| style.bg(c(CARD_HOVER)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.glass = choice;
                        window.set_background_appearance(choice.appearance());
                        this.status = format!("{} · {}", choice.label(), choice.note()).into();
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(c(TEXT))
                            .child(choice.label()),
                    )
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(c(DIM))
                            .child(choice.note()),
                    ),
            );
        }

        v_flex()
            .gap(px(18.))
            .child(
                v_flex()
                    .gap(px(14.))
                    .p(px(22.))
                    .rounded(px(22.))
                    .bg(c(CARD))
                    .border_1()
                    .border_color(c(BORDER))
                    .child(section_label("窗口材质"))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(c(MUTED))
                            .child(match self.lang {
                                Lang::Zh => "运行时切换 GPUI 的 WindowBackgroundAppearance。Win10 用 Acrylic；Mica 需要 Win11 22H2+（更老系统会静默回退）。",
                                Lang::En => "Switches GPUI's WindowBackgroundAppearance at runtime. Use Acrylic on Win10; Mica requires Win11 22H2+ (older builds silently fall back).",
                            }),
                    )
                    .child(options),
            )
            .child(
                v_flex()
                    .gap(px(8.))
                    .p(px(22.))
                    .rounded(px(22.))
                    .bg(c(CARD))
                    .border_1()
                    .border_color(c(BORDER))
                    .child(section_label("关于这个预览"))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(c(MUTED))
                            .child(match self.lang {
                                Lang::Zh => "这是 GPUI 重构的分支预览，不是默认设置界面。准星覆盖层仍是 Direct2D；旧 egui 设置界面仍可用 acaja-ui.exe 打开。",
                                Lang::En => "This is the GPUI refactor preview, not the default settings UI. The crosshair overlay remains Direct2D; the old egui settings UI is still available as acaja-ui.exe.",
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(c(DIM))
                            .child(format!(
                                "build {} · log: %APPDATA%/ACAJACrosshair/acaja-ui-gpui.log",
                                crate::VERSION
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_placeholder(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let title = t(self.lang, self.section.title_key());
        v_flex()
            .gap(px(12.))
            .p(px(26.))
            .rounded(px(24.))
            .bg(c(CARD))
            .border_1()
            .border_color(c(BORDER))
            .child(
                div()
                    .text_size(px(18.))
                    .font_family(FONT_SERIF)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(c(TEXT))
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(c(MUTED))
                    .child(match self.lang {
                        Lang::Zh => "该分区正在用 GPUI 重做。当前预览先完整实现「形状样式」与「系统」；其余分区会在后续提交里逐项迁移。",
                        Lang::En => "This section is being rebuilt in GPUI. The preview currently implements Style and System; the rest will be migrated in follow-up commits.",
                    }),
            )
            .child(
                Button::new("back-style")
                    .label(t(self.lang, "nav_style"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.section = Section::Style;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_bottom_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = crate::ipc::find_backend().is_some();
        let close_label = match self.lang {
            Lang::Zh => "关闭",
            Lang::En => "Close",
        };
        h_flex()
            .h(px(64.))
            .px(px(28.))
            .items_center()
            .justify_between()
            .bg(c(0x0A0A0DF2))
            .border_t_1()
            .border_color(c(BORDER))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(9.))
                    .child(
                        div()
                            .w(px(7.))
                            .h(px(7.))
                            .rounded_full()
                            .bg(if connected { c(SUCCESS) } else { c(WARN) }),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(c(MUTED))
                            .child(self.status.clone()),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Button::new("close")
                            .ghost()
                            .label(close_label)
                            .on_click(cx.listener(|this, _, window, _| {
                                this.save_on_drop = false;
                                window.remove_window();
                            })),
                    )
                    .child(
                        Button::new("apply")
                            .primary()
                            .label(t(self.lang, "push_apply"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.push_to_backend(true, cx);
                            })),
                    ),
            )
    }
}

// ===========================================================================
// 小部件
// ===========================================================================

fn section_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(MUTED))
        .child(label.into())
}

fn row_label(label: &'static str, hint: &'static str) -> impl IntoElement {
    h_flex()
        .items_center()
        .justify_between()
        .child(div().text_size(px(11.5)).text_color(c(MUTED)).child(label))
        .child(div().text_size(px(9.5)).text_color(c(DIM)).child(hint))
}

fn top_chip(label: &'static str, dot: u32) -> impl IntoElement {
    h_flex()
        .absolute()
        .top(px(14.))
        .left(px(14.))
        .items_center()
        .gap(px(6.))
        .h(px(22.))
        .px(px(9.))
        .rounded_full()
        .bg(ca(0x000000, 0.45))
        .border_1()
        .border_color(ca(0xFFFFFF, 0.10))
        .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(c(dot)))
        .child(
            div()
                .text_size(px(9.5))
                .text_color(c(MUTED))
                .child(label),
        )
}

fn nav_glyph(section: Section, active: bool) -> impl IntoElement {
    let color = if active { c(0xFFFFFF) } else { c(MUTED) };
    div().w(px(20.)).h(px(20.)).child(
        canvas(
            move |bounds, _, _| bounds,
            move |_, bounds, window, _| {
                paint_nav_glyph(window, bounds, section, color);
            },
        )
        .size_full(),
    )
}

fn shape_thumb(shape: CrossShape, color: Hsla) -> impl IntoElement {
    div().w(px(34.)).h(px(34.)).child(
        canvas(
            move |bounds, _, _| bounds,
            move |_, bounds, window, _| {
                paint_shape_thumb(window, bounds, shape, color);
            },
        )
        .size_full(),
    )
}

// ===========================================================================
// 自绘：预览 / 缩略图 / 图标
// ===========================================================================

fn paint_preview(window: &mut Window, bounds: Bounds<gpui_kit::Pixels>, preset: &Preset) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let cx = f32::from(bounds.center().x);
    let cy = f32::from(bounds.center().y);
    if w <= 1.0 || h <= 1.0 {
        return;
    }

    // 背景网格
    let grid = ca(0xFFFFFF, 0.035);
    let step = 32.0_f32;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mut x = left;
    while x <= left + w {
        let mut path = PathBuilder::stroke(px(1.0));
        path.move_to(point(px(x), px(top)));
        path.line_to(point(px(x), px(top + h)));
        if let Ok(path) = path.build() {
            window.paint_path(path, grid);
        }
        x += step;
    }
    let mut y = top;
    while y <= top + h {
        let mut path = PathBuilder::stroke(px(1.0));
        path.move_to(point(px(left), px(y)));
        path.line_to(point(px(left + w), px(y)));
        if let Ok(path) = path.build() {
            window.paint_path(path, grid);
        }
        y += step;
    }

    // 中心柔光
    paint_circle(window, cx, cy, (w.min(h) * 0.34).max(40.0), ca(ACCENT, 0.055), true, 0.0);
    paint_circle(window, cx, cy, (w.min(h) * 0.18).max(24.0), ca(ACCENT_2, 0.045), true, 0.0);

    let params = ShapeParams::from_preset(preset, 0.0);
    let Some(geom) = shapes::build(preset.shape, &params) else {
        // 自定义图片：画一个占位框
        paint_rect(window, cx - 34.0, cy - 24.0, 68.0, 48.0, ca(0xFFFFFF, 0.22), 1.2, false);
        paint_line(window, cx - 34.0, cy - 24.0, cx + 34.0, cy + 24.0, ca(0xFFFFFF, 0.12), 1.0);
        return;
    };

    let scale = (w.min(h) * 0.72) / 208.0;
    let theta = preset.rotation.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();
    let tf = |x: f32, y: f32| {
        (
            cx + (x * cos_t - y * sin_t) * scale,
            cy + (x * sin_t + y * cos_t) * scale,
        )
    };

    let colors = preset.active_colors();
    let main = preset.color.as_str();
    let outline_enabled = preset.outline.enabled;

    for prim in &geom.prims {
        let slot = prim_slot(prim);
        if outline_enabled {
            let color = hex_color(&preset.outline.color, preset.outline.opacity);
            paint_prim(
                window,
                prim,
                &tf,
                color,
                preset.thickness + preset.outline.thickness * 2.0,
                preset.outline.thickness,
                scale,
            );
        }
        let color = hex_color(
            match slot {
                SLOT_TOP => colors.top.as_str(),
                SLOT_BOTTOM => colors.bottom.as_str(),
                SLOT_LEFT => colors.left.as_str(),
                SLOT_RIGHT => colors.right.as_str(),
                _ => main,
            },
            preset.opacity,
        );
        paint_prim(window, prim, &tf, color, preset.thickness, 0.0, scale);
    }
}

fn paint_shape_thumb(
    window: &mut Window,
    bounds: Bounds<gpui_kit::Pixels>,
    shape: CrossShape,
    color: Hsla,
) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let cx = f32::from(bounds.center().x);
    let cy = f32::from(bounds.center().y);
    if w <= 1.0 || h <= 1.0 {
        return;
    }

    let params = ShapeParams {
        size: 12.0,
        thickness: 1.8,
        gap: 2.0,
        dot: 3.0,
        expand: 0.0,
        outline: 0.0,
    };
    let Some(geom) = shapes::build(shape, &params) else {
        paint_rect(window, cx - 11.0, cy - 8.0, 22.0, 16.0, ca(0xFFFFFF, 0.22), 1.0, false);
        paint_line(window, cx - 11.0, cy - 8.0, cx + 11.0, cy + 8.0, ca(0xFFFFFF, 0.14), 1.0);
        return;
    };

    let scale = (w.min(h) * 0.66) / 26.0;
    let tf = |x: f32, y: f32| (cx + x * scale, cy + y * scale);
    for prim in &geom.prims {
        paint_prim(window, prim, &tf, color, 1.8, 0.0, scale);
    }
}

fn paint_prim(
    window: &mut Window,
    prim: &Prim,
    tf: &dyn Fn(f32, f32) -> (f32, f32),
    color: Hsla,
    width: f32,
    delta: f32,
    scale: f32,
) {
    let width = width.max(0.6);
    match prim {
        Prim::Line { x1, y1, x2, y2, .. } => {
            let (ax, ay) = tf(*x1, *y1);
            let (bx, by) = tf(*x2, *y2);
            paint_line(window, ax, ay, bx, by, color, width + delta * 2.0);
        }
        Prim::RectFill { cx, cy, w, h, .. } => {
            let hw = (*w + delta * 2.0) / 2.0;
            let hh = (*h + delta * 2.0) / 2.0;
            let corners = [
                tf(*cx - hw, *cy - hh),
                tf(*cx + hw, *cy - hh),
                tf(*cx + hw, *cy + hh),
                tf(*cx - hw, *cy + hh),
            ];
            paint_polygon(window, &corners, color, width, true);
        }
        Prim::RectStroke { cx, cy, w, h, .. } => {
            let hw = (*w + delta * 2.0) / 2.0;
            let hh = (*h + delta * 2.0) / 2.0;
            let corners = [
                tf(*cx - hw, *cy - hh),
                tf(*cx + hw, *cy - hh),
                tf(*cx + hw, *cy + hh),
                tf(*cx - hw, *cy + hh),
            ];
            paint_polygon(window, &corners, color, width, false);
        }
        Prim::Dot { cx, cy, r, .. } => {
            let (px_, py_) = tf(*cx, *cy);
            paint_circle(window, px_, py_, (r + delta) * scale, color, true, 0.0);
        }
        Prim::Ring { cx, cy, r, .. } => {
            let (px_, py_) = tf(*cx, *cy);
            paint_circle(window, px_, py_, (r + delta) * scale, color, false, width);
        }
        Prim::PolyFill { pts, .. } => {
            if delta > 0.0 {
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    let (ax, ay) = tf(a.x, a.y);
                    let (bx, by) = tf(b.x, b.y);
                    paint_line(window, ax, ay, bx, by, color, width + delta * 2.0);
                }
            } else {
                let points: Vec<_> = pts
                    .iter()
                    .map(|pt| {
                        let (x, y) = tf(pt.x, pt.y);
                        point(px(x), px(y))
                    })
                    .collect();
                let mut path = PathBuilder::fill();
                if let Some(first) = points.first() {
                    path.move_to(*first);
                    for p in points.iter().skip(1) {
                        path.line_to(*p);
                    }
                    path.close();
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
            }
        }
    }
}

fn prim_slot(prim: &Prim) -> u8 {
    match prim {
        Prim::Line { slot, .. }
        | Prim::RectFill { slot, .. }
        | Prim::RectStroke { slot, .. }
        | Prim::Dot { slot, .. }
        | Prim::Ring { slot, .. }
        | Prim::PolyFill { slot, .. } => *slot,
    }
}

fn paint_nav_glyph(window: &mut Window, bounds: Bounds<gpui_kit::Pixels>, section: Section, color: Hsla) {
    let cx = f32::from(bounds.center().x);
    let cy = f32::from(bounds.center().y);
    let stroke = 1.4;
    match section {
        Section::Style => {
            paint_circle(window, cx, cy, 4.4, color, false, stroke);
            for (dx, dy) in [(0.0_f32, -1.0_f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                paint_line(
                    window,
                    cx + dx * 5.6,
                    cy + dy * 5.6,
                    cx + dx * 7.6,
                    cy + dy * 7.6,
                    color,
                    stroke,
                );
            }
        }
        Section::Dynamic => {
            for (i, h) in [3.5_f32, 6.5, 4.5].iter().enumerate() {
                let x = cx - 5.0 + i as f32 * 5.0;
                paint_line(window, x, cy - h * 0.5, x, cy + h * 0.5, color, stroke);
            }
        }
        Section::Position => {
            for (sx, sy) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                paint_line(window, cx + sx * 7.0, cy + sy * 2.8, cx + sx * 7.0, cy + sy * 7.0, color, stroke);
                paint_line(window, cx + sx * 2.8, cy + sy * 7.0, cx + sx * 7.0, cy + sy * 7.0, color, stroke);
            }
        }
        Section::Gamepad => {
            paint_rect(window, cx - 7.5, cy - 4.5, 15.0, 9.0, color, stroke, false);
            paint_circle(window, cx - 3.8, cy, 1.5, color, true, 0.0);
            paint_circle(window, cx + 3.8, cy, 1.5, color, true, 0.0);
        }
        Section::Hotkey => {
            paint_rect(window, cx - 7.0, cy - 5.5, 14.0, 10.0, color, stroke, false);
            paint_line(window, cx - 3.0, cy + 3.8, cx + 3.0, cy + 3.8, color, stroke);
        }
        Section::Image => {
            paint_rect(window, cx - 7.0, cy - 6.0, 14.0, 12.0, color, stroke, false);
            paint_line(window, cx - 4.0, cy + 2.5, cx, cy - 2.0, color, stroke);
            paint_line(window, cx, cy - 2.0, cx + 3.0, cy + 1.2, color, stroke);
        }
        Section::Presets => {
            paint_rect(window, cx - 8.0, cy - 6.0, 13.0, 10.0, ca(0xFFFFFF, 0.35), stroke, false);
            paint_rect(window, cx - 4.0, cy - 2.5, 13.0, 10.0, color, stroke, false);
        }
        Section::System => {
            paint_circle(window, cx, cy, 5.2, color, false, stroke);
            for (dx, dy) in [(0.0_f32, -1.0_f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
                paint_line(window, cx + dx * 6.4, cy + dy * 6.4, cx + dx * 8.2, cy + dy * 8.2, color, stroke);
            }
            paint_circle(window, cx, cy, 1.6, color, true, 0.0);
        }
    }
}

fn paint_cross_mark(window: &mut Window, cx: f32, cy: f32, r: f32, color: Hsla) {
    for (dx, dy) in [(0.0_f32, -1.0_f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
        paint_line(window, cx + dx * r * 0.25, cy + dy * r * 0.25, cx + dx * r, cy + dy * r, color, 1.8);
    }
    paint_circle(window, cx, cy, 1.8, color, true, 0.0);
}

fn paint_line(window: &mut Window, x1: f32, y1: f32, x2: f32, y2: f32, color: Hsla, width: f32) {
    let mut path = PathBuilder::stroke(px(width.max(0.5)));
    path.move_to(point(px(x1), px(y1)));
    path.line_to(point(px(x2), px(y2)));
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_rect(
    window: &mut Window,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Hsla,
    width: f32,
    fill: bool,
) {
    let mut path = if fill {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(width.max(0.5)))
    };
    path.move_to(point(px(x), px(y)));
    path.line_to(point(px(x + w), px(y)));
    path.line_to(point(px(x + w), px(y + h)));
    path.line_to(point(px(x), px(y + h)));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_polygon(
    window: &mut Window,
    points: &[(f32, f32)],
    color: Hsla,
    width: f32,
    fill: bool,
) {
    let Some(&(x0, y0)) = points.first() else {
        return;
    };
    let mut path = if fill {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(width.max(0.5)))
    };
    path.move_to(point(px(x0), px(y0)));
    for &(x, y) in points.iter().skip(1) {
        path.line_to(point(px(x), px(y)));
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_circle(
    window: &mut Window,
    cx: f32,
    cy: f32,
    r: f32,
    color: Hsla,
    fill: bool,
    width: f32,
) {
    if r <= 0.0 {
        return;
    }
    let mut path = if fill {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px(width.max(0.5)))
    };
    let k = 0.552_284_8 * r;
    path.move_to(point(px(cx + r), px(cy)));
    path.cubic_bezier_to(
        point(px(cx), px(cy + r)),
        point(px(cx + r), px(cy + k)),
        point(px(cx + k), px(cy + r)),
    );
    path.cubic_bezier_to(
        point(px(cx - r), px(cy)),
        point(px(cx - k), px(cy + r)),
        point(px(cx - r), px(cy + k)),
    );
    path.cubic_bezier_to(
        point(px(cx), px(cy - r)),
        point(px(cx - r), px(cy - k)),
        point(px(cx - k), px(cy - r)),
    );
    path.cubic_bezier_to(
        point(px(cx + r), px(cy)),
        point(px(cx + k), px(cy - r)),
        point(px(cx + r), px(cy - k)),
    );
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
