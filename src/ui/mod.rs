//! ACAJA 设置界面（GPUI Kit，v1.3 起为默认设置进程）。
//!
//! 设计目标：现代、克制、可读的深色界面。窗口级使用系统材质（Acrylic / Mica /
//! Transparent / Opaque），窗口内部用半透明表面、1px 镜面边和分层色块构造玻璃感；
//! GPUI 目前没有逐元素 backdrop blur（gpui-kit#3598），这是上限而不是实现取巧。
//!
//! 功能与旧 egui 设置界面一一对应：
//! - 形状样式：22 形状、实时预览、颜色/四象限、大小/粗细/透明度/旋转、
//!   空心缺口/中心点、描边、8 套模板
//! - 动态准星：开火扩散、恢复速度、后坐力指示、扩散形态预览
//! - 位置：多显示器、跟随前台、X/Y、居中、吸附
//! - 手柄：ADS 模式、触发键、阈值、开火扩散
//! - 快捷键：录制/清空/手动输入、切换下一预设、右键模式
//! - 图片：选择/清除、缩略图、缩放
//! - 预设：保存、导入/导出、创建、复制、重命名、删除、游戏绑定
//! - 系统：开机自启、窗口材质、玻璃强度、退出主程序
//!
//! 数据链路复用 `PresetStore` / `WM_COPYDATA`；滑杆拖动实时推送（120ms 节流），
//! 「应用设置到主程序」保存并推送，标题栏 X 关闭时自动保存，底部「关闭」不保存退出。

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Sizable as _, Theme, ThemeMode, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, PathBuilder, Render, Rgba, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, canvas, div, img, point, px, rgba as gpui_rgba, size,
};

use crate::config::{
    AdsButton, AdsMode, GameBinding, GlassMode, Hotkey, PosVal, Preset, PresetStore, QuadColors,
    RightClickMode, Shape as CrossShape, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
    validate_preset_name,
};
use crate::i18n::Lang;
use crate::overlay::parse_hex;
use crate::overlay::shapes::{self, Prim, ShapeParams, SLOT_BOTTOM, SLOT_LEFT, SLOT_RIGHT, SLOT_TOP};
use crate::strings::{ads_mode_name, shape_name, t};
use crate::system::monitor::MonitorInfo;

// ===========================================================================
// 设计令牌
// ===========================================================================

const FONT_SANS: &str = "Noto Sans SC Thin";
const FONT_SERIF: &str = "Noto Serif SC ExtraLight";

const TEXT: u32 = 0xF4F5F8FF;
const MUTED: u32 = 0xA7ABB8FF;
const DIM: u32 = 0x71747FFF;
const ACCENT: u32 = 0x7C6CF0FF;
const ACCENT_2: u32 = 0x4F7DFFFF;
const SUCCESS: u32 = 0x4ADE80FF;
const WARN: u32 = 0xF5A524FF;
const DANGER: u32 = 0xFF5C6CFF;

const COLOR_PRESETS: [&str; 6] = ["#FF3B30", "#FFD60A", "#30D158", "#64D2FF", "#BF5AF2", "#FFFFFF"];

fn c(value: u32) -> Hsla {
    Hsla::from(gpui_rgba(value))
}

fn ca(value: u32, alpha: f32) -> Hsla {
    Hsla::from(Rgba { a: alpha.clamp(0.0, 1.0), ..gpui_rgba(value) })
}

fn hex_color(hex: &str, alpha: f32) -> Hsla {
    let (r, g, b) = parse_hex(hex);
    Hsla::from(Rgba { r, g, b, a: alpha.clamp(0.0, 1.0) })
}

/// 表面色随「玻璃强度」变化：值越大越透（0.3 较实 / 1.0 较透）。
#[derive(Clone, Copy)]
struct Palette {
    root: Hsla,
    title_bar: Hsla,
    sidebar: Hsla,
    card: Hsla,
    card_hover: Hsla,
    control: Hsla,
    border: Hsla,
    text: Hsla,
    muted: Hsla,
    dim: Hsla,
    accent: Hsla,
    accent2: Hsla,
    accent_soft: Hsla,
    success: Hsla,
    warn: Hsla,
}

impl Palette {
    fn from_level(level: f32) -> Self {
        let level = level.clamp(0.3, 1.0);
        let k = (level - 0.3) / 0.7; // 0 = 最实, 1 = 最透
        Self {
            root: ca(0x08080A, 0.92 - 0.18 * k),
            title_bar: ca(0x0B0B0F, 0.94 - 0.12 * k),
            sidebar: ca(0x0B0B0F, 0.90 - 0.16 * k),
            card: ca(0x14141A, 0.97 - 0.09 * k),
            card_hover: ca(0x1B1B24, 0.98 - 0.08 * k),
            control: ca(0x101015, 0.95 - 0.10 * k),
            border: ca(0x2A2A36, 0.80),
            text: c(TEXT),
            muted: c(MUTED),
            dim: c(DIM),
            accent: c(ACCENT),
            accent2: c(ACCENT_2),
            accent_soft: ca(ACCENT, 0.16),
            success: c(SUCCESS),
            warn: c(WARN),
        }
    }
}

// ===========================================================================
// 分区 / 材质
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

    fn subtitle(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Section::Style, Lang::Zh) => "为准星调校每一像素，改动实时同步到屏幕。",
            (Section::Style, Lang::En) => "Tune every pixel — changes reach the screen live.",
            (Section::Dynamic, Lang::Zh) => "开火扩散与恢复速度，让准星跟着枪口呼吸。",
            (Section::Dynamic, Lang::En) => "Fire spread and recovery — let the crosshair breathe.",
            (Section::Position, Lang::Zh) => "多显示器、像素偏移与前台窗口吸附。",
            (Section::Position, Lang::En) => "Multi-monitor placement, pixel offsets and foreground snapping.",
            (Section::Gamepad, Lang::Zh) => "XInput ADS 联动：瞄准时隐藏或显示准星。",
            (Section::Gamepad, Lang::En) => "XInput ADS integration: hide or show while aiming.",
            (Section::Hotkey, Lang::Zh) => "全局热键与右键模式。",
            (Section::Hotkey, Lang::En) => "Global hotkeys and right-click behavior.",
            (Section::Image, Lang::Zh) => "用 PNG / JPG 替换几何准星。",
            (Section::Image, Lang::En) => "Replace the vector crosshair with a PNG / JPG.",
            (Section::Presets, Lang::Zh) => "预设、导入导出、按游戏自动切换。",
            (Section::Presets, Lang::En) => "Presets, import/export and per-game auto switching.",
            (Section::System, Lang::Zh) => "开机自启、窗口材质与主程序控制。",
            (Section::System, Lang::En) => "Autostart, window material and backend control.",
        }
    }
}

/// GPUI 可用的窗口材质。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GpuiGlass {
    Acrylic,
    Mica,
    Transparent,
    Opaque,
}

impl GpuiGlass {
    const ALL: [GpuiGlass; 4] =
        [GpuiGlass::Acrylic, GpuiGlass::Mica, GpuiGlass::Transparent, GpuiGlass::Opaque];

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
            GpuiGlass::Transparent => "alpha only, no blur",
            GpuiGlass::Opaque => "safe fallback",
        }
    }

    fn from_mode(mode: GlassMode) -> Self {
        match mode {
            GlassMode::Acrylic | GlassMode::Auto => GpuiGlass::Acrylic,
            GlassMode::Backdrop | GlassMode::Mica => GpuiGlass::Mica,
            GlassMode::Transparent => GpuiGlass::Transparent,
            GlassMode::Off => GpuiGlass::Opaque,
        }
    }

    fn to_mode(self) -> GlassMode {
        match self {
            GpuiGlass::Acrylic => GlassMode::Acrylic,
            GpuiGlass::Mica => GlassMode::Mica,
            GpuiGlass::Transparent => GlassMode::Transparent,
            GpuiGlass::Opaque => GlassMode::Off,
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

            let initial_glass = {
                let store = store.lock();
                if glass_test {
                    GpuiGlass::Acrylic
                } else {
                    GpuiGlass::from_mode(store.app.glass.mode)
                }
            };
            let store_for_view = store.clone();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1240.), px(800.)), cx)),
                window_min_size: Some(size(px(1080.), px(700.))),
                window_background: initial_glass.appearance(),
                app_id: Some("acaja".into()),
                ..TitleBar::window_options()
            };

            let result = gpui_kit::open_window(options, cx, move |window, cx| {
                window.activate_window();
                window.set_window_title("ACAJA");
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
    log::info!("GPUI 可用字体: {:?}", cx.text_system().all_font_names());
}

fn configure_theme(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.primary = c(ACCENT);
        theme.primary_foreground = c(0xFFFFFFFF);
        theme.background = c(0x08080AFF);
        theme.foreground = c(TEXT);
        theme.border = ca(0x2A2A36, 0.80);
        theme.muted = c(0x101015FF);
        theme.muted_foreground = c(MUTED);
        theme.secondary = ca(0x14141A, 0.97);
        theme.secondary_hover = ca(0x1B1B24, 0.98);
        theme.secondary_active = ca(0x22222C, 0.98);
        theme.danger = c(DANGER);
        theme.danger_foreground = c(0xFFFFFFFF);
        theme.danger_active = c(0xE04455FF);
        theme.title_bar = ca(0x0B0B0F, 0.94);
        theme.title_bar_border = ca(0x2A2A36, 0.80);
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
    glass_level: f32,
    last_push: Instant,
    last_push_body: String,
    save_on_drop: bool,

    // 文本输入
    color_inputs: Vec<Entity<InputState>>,
    hotkey_inputs: Vec<Entity<InputState>>,
    hotkey_focus: Vec<FocusHandle>,
    new_preset_input: Entity<InputState>,
    rename_input: Entity<InputState>,
    binding_exe_input: Entity<InputState>,
    pos_x_input: Entity<InputState>,
    pos_y_input: Entity<InputState>,

    // 滑杆
    size_slider: Entity<SliderState>,
    thickness_slider: Entity<SliderState>,
    opacity_slider: Entity<SliderState>,
    rotation_slider: Entity<SliderState>,
    gap_slider: Entity<SliderState>,
    dot_slider: Entity<SliderState>,
    outline_thickness_slider: Entity<SliderState>,
    outline_opacity_slider: Entity<SliderState>,
    dynamic_expand_slider: Entity<SliderState>,
    dynamic_recover_slider: Entity<SliderState>,
    threshold_slider: Entity<SliderState>,
    image_scale_slider: Entity<SliderState>,
    glass_level_slider: Entity<SliderState>,

    // 预设 / 绑定
    presets: Vec<String>,
    rename_from: Option<String>,
    rename_confirm: bool,
    delete_confirm: Option<String>,
    binding_preset: String,

    // 显示器
    monitors: Vec<MonitorInfo>,
    monitors_at: Option<Instant>,

    // 热键录制
    recording: Option<usize>,

    // 模板缩略图
    tpl_previews: Vec<Preset>,

    _subscriptions: Vec<Subscription>,
}

impl SettingsApp {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        store: Arc<Mutex<PresetStore>>,
        glass_test: bool,
    ) -> Self {
        let (draft, preset_name, lang, stored_mode, stored_level) = {
            let store = store.lock();
            (
                store.get_active().clone(),
                store.active_name(),
                store.app.lang(),
                store.app.glass.mode,
                store.app.glass.level_clamped(),
            )
        };
        let glass = if glass_test {
            GpuiGlass::Acrylic
        } else {
            GpuiGlass::from_mode(stored_mode)
        };
        window.set_background_appearance(glass.appearance());
        log::info!("窗口材质: {}", glass.label());

        let color_values = [
            draft.color.clone(),
            draft.colors.top.clone(),
            draft.colors.bottom.clone(),
            draft.colors.left.clone(),
            draft.colors.right.clone(),
            draft.outline.color.clone(),
        ];
        let mut color_inputs = Vec::new();
        for value in color_values {
            color_inputs.push(cx.new(|cx| InputState::new(window, cx).default_value(value)));
        }

        let hotkey_values = [draft.hotkey_toggle.to_string(), draft.hotkey_next_profile.to_string()];
        let mut hotkey_inputs = Vec::new();
        for value in hotkey_values {
            hotkey_inputs.push(cx.new(|cx| InputState::new(window, cx).default_value(value)));
        }
        let hotkey_focus = vec![cx.focus_handle(), cx.focus_handle()];

        let new_preset_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t(lang, "new_name")));
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder(t(lang, "rename")));
        let binding_exe_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t(lang, "binding_exe")));
        let pos_x_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(pos_string(&draft.position.x))
        });
        let pos_y_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(pos_string(&draft.position.y))
        });

        let size_slider = cx.new(|_| {
            SliderState::new().min(1.0).max(200.0).step(1.0).default_value(draft.size)
        });
        let thickness_slider = cx.new(|_| {
            SliderState::new().min(0.2).max(20.0).step(0.1).default_value(draft.thickness)
        });
        let opacity_slider = cx.new(|_| {
            SliderState::new().min(0.05).max(1.0).step(0.01).default_value(draft.opacity)
        });
        let rotation_slider = cx.new(|_| {
            SliderState::new().min(0.0).max(360.0).step(1.0).default_value(draft.rotation)
        });
        let gap_slider = cx.new(|_| {
            SliderState::new().min(0.0).max(80.0).step(0.5).default_value(draft.hollow.gap)
        });
        let dot_slider = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(30.0)
                .step(0.5)
                .default_value(draft.hollow.center_dot_size)
        });
        let outline_thickness_slider = cx.new(|_| {
            SliderState::new()
                .min(0.5)
                .max(10.0)
                .step(0.1)
                .default_value(draft.outline.thickness)
        });
        let outline_opacity_slider = cx.new(|_| {
            SliderState::new()
                .min(0.1)
                .max(1.0)
                .step(0.01)
                .default_value(draft.outline.opacity)
        });
        let dynamic_expand_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(80.0)
                .step(0.5)
                .default_value(draft.dynamic.fire_expand_px)
        });
        let dynamic_recover_slider = cx.new(|_| {
            SliderState::new()
                .min(20.0)
                .max(500.0)
                .step(5.0)
                .default_value(draft.dynamic.recover_ms as f32)
        });
        let threshold_slider = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(255.0)
                .step(1.0)
                .default_value(draft.gamepad.trigger_threshold as f32)
        });
        let image_scale_slider = cx.new(|_| {
            SliderState::new().min(0.1).max(5.0).step(0.05).default_value(draft.image.scale)
        });
        let glass_level_slider = cx.new(|_| {
            SliderState::new().min(0.3).max(1.0).step(0.01).default_value(stored_level)
        });

        let mut subscriptions = Vec::new();
        for index in 0..6 {
            subscriptions.push(subscribe_text(
                cx,
                window,
                &color_inputs[index],
                move |this, text| this.on_color_input(index, text),
            ));
        }
        for slot in 0..2 {
            subscriptions.push(subscribe_text(
                cx,
                window,
                &hotkey_inputs[slot],
                move |this, text| this.on_hotkey_input(slot, text),
            ));
        }
        subscriptions.push(subscribe_text(cx, window, &pos_x_input, |this, text| {
            this.on_pos_input(true, text)
        }));
        subscriptions.push(subscribe_text(cx, window, &pos_y_input, |this, text| {
            this.on_pos_input(false, text)
        }));
        // 重命名输入只用于刷新校验提示，不直接改预设
        subscriptions.push(cx.subscribe_in(
            &rename_input,
            window,
            |_this, _state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        ));

        subscriptions.push(subscribe_slider(cx, window, &size_slider, |this, v| {
            let v = v.clamp(1.0, 200.0);
            if (this.draft.size - v).abs() < 0.001 {
                return false;
            }
            this.draft.size = v;
            true
        }));
        subscriptions.push(subscribe_slider(cx, window, &thickness_slider, |this, v| {
            let v = v.clamp(0.2, 20.0);
            if (this.draft.thickness - v).abs() < 0.001 {
                return false;
            }
            this.draft.thickness = v;
            true
        }));
        subscriptions.push(subscribe_slider(cx, window, &opacity_slider, |this, v| {
            let v = v.clamp(0.05, 1.0);
            if (this.draft.opacity - v).abs() < 0.0001 {
                return false;
            }
            this.draft.opacity = v;
            true
        }));
        subscriptions.push(subscribe_slider(cx, window, &rotation_slider, |this, v| {
            let v = v.clamp(0.0, 360.0);
            if (this.draft.rotation - v).abs() < 0.001 {
                return false;
            }
            this.draft.rotation = v;
            true
        }));
        subscriptions.push(subscribe_slider(cx, window, &gap_slider, |this, v| {
            let v = v.clamp(0.0, 80.0);
            if (this.draft.hollow.gap - v).abs() < 0.001 {
                return false;
            }
            this.draft.hollow.gap = v;
            true
        }));
        subscriptions.push(subscribe_slider(cx, window, &dot_slider, |this, v| {
            let v = v.clamp(1.0, 30.0);
            if (this.draft.hollow.center_dot_size - v).abs() < 0.001 {
                return false;
            }
            this.draft.hollow.center_dot_size = v;
            true
        }));
        subscriptions.push(subscribe_slider(
            cx,
            window,
            &outline_thickness_slider,
            |this, v| {
                let v = v.clamp(0.5, 10.0);
                if (this.draft.outline.thickness - v).abs() < 0.001 {
                    return false;
                }
                this.draft.outline.thickness = v;
                true
            },
        ));
        subscriptions.push(subscribe_slider(
            cx,
            window,
            &outline_opacity_slider,
            |this, v| {
                let v = v.clamp(0.1, 1.0);
                if (this.draft.outline.opacity - v).abs() < 0.0001 {
                    return false;
                }
                this.draft.outline.opacity = v;
                true
            },
        ));
        subscriptions.push(subscribe_slider(
            cx,
            window,
            &dynamic_expand_slider,
            |this, v| {
                let v = v.clamp(0.0, 80.0);
                if (this.draft.dynamic.fire_expand_px - v).abs() < 0.001 {
                    return false;
                }
                this.draft.dynamic.fire_expand_px = v;
                true
            },
        ));
        subscriptions.push(subscribe_slider(
            cx,
            window,
            &dynamic_recover_slider,
            |this, v| {
                let v = v.round().clamp(20.0, 500.0) as u32;
                if this.draft.dynamic.recover_ms == v {
                    return false;
                }
                this.draft.dynamic.recover_ms = v;
                true
            },
        ));
        subscriptions.push(subscribe_slider(cx, window, &threshold_slider, |this, v| {
            let v = v.round().clamp(0.0, 255.0) as u8;
            if this.draft.gamepad.trigger_threshold == v {
                return false;
            }
            this.draft.gamepad.trigger_threshold = v;
            true
        }));
        subscriptions.push(subscribe_slider(cx, window, &image_scale_slider, |this, v| {
            let v = v.clamp(0.1, 5.0);
            if (this.draft.image.scale - v).abs() < 0.0001 {
                return false;
            }
            this.draft.image.scale = v;
            true
        }));
        subscriptions.push(cx.subscribe_in(
            &glass_level_slider,
            window,
            |this, _state, event, _, cx| {
                if let SliderEvent::Change(value) = event {
                    this.glass_level = value.start().clamp(0.3, 1.0);
                    cx.notify();
                }
            },
        ));

        let presets = store.lock().preset_names();
        let monitors = crate::system::monitor::monitors();
        let tpl_previews = TEMPLATES
            .iter()
            .map(|(_, apply)| {
                let mut p = template_base(&draft);
                apply(&mut p);
                p
            })
            .collect();

        let connected = crate::ipc::find_backend().is_some();
        let status = if connected {
            t(lang, "backend_connected")
        } else {
            t(lang, "backend_file_mode")
        };

        Self {
            store,
            draft,
            preset_name: preset_name.clone(),
            section: Section::Style,
            lang,
            status: status.into(),
            glass,
            glass_level: stored_level,
            last_push: Instant::now() - Duration::from_secs(1),
            last_push_body: String::new(),
            save_on_drop: true,
            color_inputs,
            hotkey_inputs,
            hotkey_focus,
            new_preset_input,
            rename_input,
            binding_exe_input,
            pos_x_input,
            pos_y_input,
            size_slider,
            thickness_slider,
            opacity_slider,
            rotation_slider,
            gap_slider,
            dot_slider,
            outline_thickness_slider,
            outline_opacity_slider,
            dynamic_expand_slider,
            dynamic_recover_slider,
            threshold_slider,
            image_scale_slider,
            glass_level_slider,
            presets,
            rename_from: None,
            rename_confirm: false,
            delete_confirm: None,
            binding_preset: preset_name,
            monitors,
            monitors_at: Some(Instant::now()),
            recording: None,
            tpl_previews,
            _subscriptions: subscriptions,
        }
    }

    fn palette(&self) -> Palette {
        Palette::from_level(self.glass_level)
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
        let body = crate::ipc::preset_payload(&self.draft, true);
        if body == self.last_push_body {
            cx.notify();
            return;
        }
        self.last_push = now;
        if let Some(hwnd) = crate::ipc::find_backend() {
            let ok = crate::ipc::send_json_timeout(
                hwnd,
                crate::ipc::IPC_TAG_PRESET,
                &body,
                120,
            );
            if ok {
                self.last_push_body = body;
            }
        }
        cx.notify();
    }

    fn push_to_backend(&mut self, save: bool, cx: &mut Context<Self>) {
        let mut message: Option<String> = None;

        if save {
            let name = self.preset_name.clone();
            let preset = self.draft.clone();
            let result = {
                let mut store = self.store.lock();
                let r = store.save_preset(&name, &preset);
                if r.is_ok() {
                    let _ = store.save_app();
                }
                r
            };
            if let Err(err) = result {
                log::warn!("保存预设失败: {err}");
                message = Some(format!("{}: {err}", t(self.lang, "error")));
            }
        }

        if message.is_none() {
            if let Some(hwnd) = crate::ipc::find_backend() {
                let body = crate::ipc::preset_payload(&self.draft, true);
                let ok = crate::ipc::send_json_timeout(
                    hwnd,
                    crate::ipc::IPC_TAG_PRESET,
                    &body,
                    250,
                );
                if ok {
                    self.last_push_body = body;
                    self.last_push = Instant::now();
                }
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

    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let colors = [
            self.draft.color.clone(),
            self.draft.colors.top.clone(),
            self.draft.colors.bottom.clone(),
            self.draft.colors.left.clone(),
            self.draft.colors.right.clone(),
            self.draft.outline.color.clone(),
        ];
        for (index, value) in colors.into_iter().enumerate() {
            self.color_inputs[index].update(cx, |state, cx| {
                state.set_value(value, window, cx);
            });
        }

        let hotkeys = [
            self.draft.hotkey_toggle.to_string(),
            self.draft.hotkey_next_profile.to_string(),
        ];
        for (index, value) in hotkeys.into_iter().enumerate() {
            self.hotkey_inputs[index].update(cx, |state, cx| {
                state.set_value(value, window, cx);
            });
        }

        let x = pos_string(&self.draft.position.x);
        let y = pos_string(&self.draft.position.y);
        self.pos_x_input.update(cx, |state, cx| state.set_value(x, window, cx));
        self.pos_y_input.update(cx, |state, cx| state.set_value(y, window, cx));

        let size = self.draft.size;
        let thickness = self.draft.thickness;
        let opacity = self.draft.opacity;
        let rotation = self.draft.rotation;
        let gap = self.draft.hollow.gap;
        let dot = self.draft.hollow.center_dot_size;
        let outline_thickness = self.draft.outline.thickness;
        let outline_opacity = self.draft.outline.opacity;
        let expand = self.draft.dynamic.fire_expand_px;
        let recover = self.draft.dynamic.recover_ms as f32;
        let threshold = self.draft.gamepad.trigger_threshold as f32;
        let image_scale = self.draft.image.scale;
        let glass_level = self.glass_level;

        self.size_slider.update(cx, |s, cx| s.set_value(size, window, cx));
        self.thickness_slider.update(cx, |s, cx| s.set_value(thickness, window, cx));
        self.opacity_slider.update(cx, |s, cx| s.set_value(opacity, window, cx));
        self.rotation_slider.update(cx, |s, cx| s.set_value(rotation, window, cx));
        self.gap_slider.update(cx, |s, cx| s.set_value(gap, window, cx));
        self.dot_slider.update(cx, |s, cx| s.set_value(dot, window, cx));
        self.outline_thickness_slider.update(cx, |s, cx| {
            s.set_value(outline_thickness, window, cx)
        });
        self.outline_opacity_slider.update(cx, |s, cx| {
            s.set_value(outline_opacity, window, cx)
        });
        self.dynamic_expand_slider.update(cx, |s, cx| s.set_value(expand, window, cx));
        self.dynamic_recover_slider.update(cx, |s, cx| s.set_value(recover, window, cx));
        self.threshold_slider.update(cx, |s, cx| s.set_value(threshold, window, cx));
        self.image_scale_slider.update(cx, |s, cx| s.set_value(image_scale, window, cx));
        self.glass_level_slider.update(cx, |s, cx| s.set_value(glass_level, window, cx));
    }

    fn apply_preset(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let loaded = {
            let mut store = self.store.lock();
            let Some(preset) = store.get(name).cloned() else {
                return;
            };
            store.activate(name);
            let _ = store.save_app();
            preset
        };
        self.draft = loaded;
        self.preset_name = name.to_string();
        self.presets = self.store.lock().preset_names();
        self.binding_preset = name.to_string();
        self.recording = None;
        self.rename_from = None;
        self.rename_confirm = false;
        self.delete_confirm = None;
        self.sync_inputs(window, cx);
        self.save_on_drop = true;
        self.mark_dirty(cx);
    }

    fn save_current(&mut self, cx: &mut Context<Self>) {
        let name = self.preset_name.clone();
        let preset = self.draft.clone();
        let result = {
            let mut store = self.store.lock();
            let r = store.save_preset(&name, &preset);
            if r.is_ok() {
                let _ = store.save_app();
            }
            r
        };
        match result {
            Ok(()) => {
                self.status = format!("{} {}", t(self.lang, "saved"), name).into();
            }
            Err(err) => {
                log::warn!("保存预设失败: {err}");
                self.status = format!("{}: {err}", t(self.lang, "error")).into();
            }
        }
        cx.notify();
    }

    fn create_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_preset_input.read(cx).value().trim().to_string();
        if let Err(err) = validate_preset_name(&name) {
            self.status = err.to_string().into();
            cx.notify();
            return;
        }
        let preset = self.draft.clone();
        let result = {
            let mut store = self.store.lock();
            let r = store.save_preset(&name, &preset);
            if r.is_ok() {
                store.activate(&name);
                let _ = store.save_app();
            }
            r
        };
        match result {
            Ok(()) => {
                self.preset_name = name.clone();
                self.binding_preset = name.clone();
                self.new_preset_input.update(cx, |state, cx| {
                    state.set_value("", window, cx);
                });
                self.refresh_presets();
                self.status = format!("{} {}", t(self.lang, "created"), name).into();
                self.mark_dirty(cx);
            }
            Err(err) => {
                log::warn!("创建预设失败: {err}");
                self.status = format!("{}: {err}", t(self.lang, "error")).into();
                cx.notify();
            }
        }
    }

    fn import_preset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = crate::system::filedialog::pick_preset_json() else {
            return;
        };
        let result = { self.store.lock().import_preset(&path, None) };
        match result {
            Ok(name) => {
                self.refresh_presets();
                self.apply_preset(&name, window, cx);
                self.status = format!("{} {}", t(self.lang, "imported"), name).into();
                cx.notify();
            }
            Err(err) => {
                log::warn!("导入预设失败: {err}");
                self.status = format!("{}: {err}", t(self.lang, "import_failed")).into();
                cx.notify();
            }
        }
    }

    fn duplicate_preset(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let result = { self.store.lock().duplicate_preset(name) };
        match result {
            Ok(new_name) => {
                self.refresh_presets();
                self.apply_preset(&new_name, window, cx);
                self.status = format!("{} {}", t(self.lang, "duplicate"), new_name).into();
                cx.notify();
            }
            Err(err) => {
                log::warn!("复制预设失败: {err}");
                self.status = format!("{}: {err}", t(self.lang, "duplicate_failed")).into();
                cx.notify();
            }
        }
    }

    fn export_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(path) = crate::system::filedialog::save_preset_json(name) else {
            return;
        };
        let result = { self.store.lock().export_preset(name, &path) };
        match result {
            Ok(()) => {
                let file = path.file_name().and_then(|s| s.to_str()).unwrap_or("preset.json");
                self.status = format!("{} {}", t(self.lang, "exported"), file).into();
            }
            Err(err) => {
                log::warn!("导出预设失败: {err}");
                self.status = format!("{}: {err}", t(self.lang, "export_failed")).into();
            }
        }
        cx.notify();
    }

    fn start_rename(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_from = Some(name.to_string());
        self.rename_confirm = false;
        self.delete_confirm = None;
        self.rename_input.update(cx, |state, cx| {
            state.set_value(name.to_string(), window, cx);
        });
        cx.notify();
    }

    fn confirm_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(from) = self.rename_from.clone() else {
            return;
        };
        let to = self.rename_input.read(cx).value().trim().to_string();
        let taken = self.presets.iter().any(|name| name == &to);
        if validate_preset_name(&to).is_err() || to == from || taken {
            self.status = t(self.lang, "rename_failed").into();
            cx.notify();
            return;
        }
        let result = { self.store.lock().rename_preset(&from, &to) };
        match result {
            Ok(true) => {
                if self.preset_name == from {
                    self.preset_name = to.clone();
                    self.binding_preset = to.clone();
                }
                self.rename_from = None;
                self.rename_confirm = false;
                self.refresh_presets();
                self.status = format!("{} {}", t(self.lang, "renamed"), to).into();
            }
            Ok(false) => {
                self.status = t(self.lang, "rename_failed").into();
            }
            Err(err) => {
                log::warn!("重命名失败: {err}");
                self.status = format!("{}: {err}", t(self.lang, "rename_failed")).into();
            }
        }
        let _ = window;
        cx.notify();
    }

    fn request_delete(&mut self, name: &str, cx: &mut Context<Self>) {
        if name == "default" {
            self.status = t(self.lang, "cannot_delete_default").into();
        } else {
            self.delete_confirm = Some(name.to_string());
            self.rename_from = None;
        }
        cx.notify();
    }

    fn confirm_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.delete_confirm.clone() else {
            return;
        };
        let result = {
            let mut store = self.store.lock();
            let ok = store.delete_preset(&name).unwrap_or(false);
            if ok {
                let _ = store.save_app();
            }
            ok
        };
        self.delete_confirm = None;
        if result {
            self.refresh_presets();
            if self.preset_name == name {
                self.apply_preset("default", window, cx);
            }
            self.status = format!("{} {}", t(self.lang, "deleted"), name).into();
        } else {
            self.status = t(self.lang, "error").into();
        }
        cx.notify();
    }

    fn add_binding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let exe = self.binding_exe_input.read(cx).value().trim().to_ascii_lowercase();
        if exe.is_empty() {
            self.status = t(self.lang, "error").into();
            cx.notify();
            return;
        }
        let preset = if self.binding_preset.is_empty() {
            self.preset_name.clone()
        } else {
            self.binding_preset.clone()
        };
        let saved = {
            let mut store = self.store.lock();
            store.app.game_bindings.retain(|b| !b.exe.eq_ignore_ascii_case(&exe));
            store.app.game_bindings.push(GameBinding { exe, preset });
            store.save_app().is_ok()
        };
        self.binding_exe_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.status = t(self.lang, if saved { "binding_added" } else { "error" }).into();
        cx.notify();
    }

    fn remove_binding(&mut self, index: usize, cx: &mut Context<Self>) {
        let saved = {
            let mut store = self.store.lock();
            if index < store.app.game_bindings.len() {
                store.app.game_bindings.remove(index);
            }
            store.save_app().is_ok()
        };
        self.status = t(self.lang, if saved { "binding_removed" } else { "error" }).into();
        cx.notify();
    }

    fn toggle_autostart(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let target = crate::ipc::find_backend()
            .and_then(crate::system::foreground::window_process_path);
        let applied = match target {
            Some(path) => crate::system::autostart::set_autostart(enabled, Path::new(&path)).is_ok(),
            None => false,
        };
        let actual = if applied { enabled } else { !enabled };
        {
            let mut store = self.store.lock();
            store.app.autostart = actual;
            let _ = store.save_app();
        }
        self.status = t(self.lang, if applied { "saved" } else { "autostart_failed" }).into();
        cx.notify();
    }

    fn set_lang(&mut self, lang: Lang, cx: &mut Context<Self>) {
        self.lang = lang;
        {
            let mut store = self.store.lock();
            store.app.language = lang.code().to_string();
            let _ = store.save_app();
        }
        let connected = crate::ipc::find_backend().is_some();
        self.status = if connected {
            t(lang, "backend_connected")
        } else {
            t(lang, "backend_file_mode")
        }
        .into();
        cx.notify();
    }

    fn on_color_input(&mut self, index: usize, text: String) -> bool {
        let Some(hex) = normalize_hex(&text) else {
            return false;
        };
        let target = match index {
            0 => &mut self.draft.color,
            1 => &mut self.draft.colors.top,
            2 => &mut self.draft.colors.bottom,
            3 => &mut self.draft.colors.left,
            4 => &mut self.draft.colors.right,
            5 => &mut self.draft.outline.color,
            _ => return false,
        };
        if *target == hex {
            return false;
        }
        *target = hex;
        if index == 0 && !self.draft.multicolor {
            self.draft.colors = QuadColors::from_single(&self.draft.color);
        }
        true
    }

    fn set_color_value(&mut self, index: usize, hex: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hex) = normalize_hex(hex) else {
            return;
        };
        let target = match index {
            0 => &mut self.draft.color,
            1 => &mut self.draft.colors.top,
            2 => &mut self.draft.colors.bottom,
            3 => &mut self.draft.colors.left,
            4 => &mut self.draft.colors.right,
            5 => &mut self.draft.outline.color,
            _ => return,
        };
        *target = hex.clone();
        if index == 0 && !self.draft.multicolor {
            self.draft.colors = QuadColors::from_single(&hex);
        }
        self.color_inputs[index].update(cx, |state, cx| {
            state.set_value(hex, window, cx);
        });
        self.mark_dirty(cx);
    }

    fn on_hotkey_input(&mut self, slot: usize, text: String) -> bool {
        let hotkey = if text.trim().is_empty() {
            Hotkey::default()
        } else {
            match Hotkey::parse(&text) {
                Some(hotkey) => hotkey,
                None => return false,
            }
        };
        let target = if slot == 0 {
            &mut self.draft.hotkey_toggle
        } else {
            &mut self.draft.hotkey_next_profile
        };
        if *target == hotkey {
            return false;
        }
        *target = hotkey;
        true
    }

    fn on_pos_input(&mut self, x_axis: bool, text: String) -> bool {
        let trimmed = text.trim();
        let target = if x_axis {
            &mut self.draft.position.x
        } else {
            &mut self.draft.position.y
        };
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("center") {
            if *target == PosVal::Center {
                return false;
            }
            *target = PosVal::Center;
            return true;
        }
        let Ok(value) = trimmed.parse::<f32>() else {
            return false;
        };
        if *target == PosVal::Px(value) {
            return false;
        }
        *target = PosVal::Px(value);
        true
    }

    fn set_multicolor(&mut self, value: bool, window: &mut Window, cx: &mut Context<Self>) {
        if value && self.draft.colors.is_uniform() {
            let color = self.draft.color.clone();
            self.draft.colors = QuadColors::from_single(&color);
        }
        self.draft.multicolor = value;
        self.sync_inputs(window, cx);
        self.mark_dirty(cx);
    }

    fn set_outline(&mut self, value: bool, cx: &mut Context<Self>) {
        self.draft.outline.enabled = value;
        self.mark_dirty(cx);
    }

    fn set_hotkey(&mut self, slot: usize, hotkey: Hotkey) {
        if slot == 0 {
            self.draft.hotkey_toggle = hotkey;
        } else {
            self.draft.hotkey_next_profile = hotkey;
        }
    }

    fn sync_hotkey_input(&self, slot: usize, window: &mut Window, cx: &mut Context<Self>) {
        let text = if slot == 0 {
            self.draft.hotkey_toggle.to_string()
        } else {
            self.draft.hotkey_next_profile.to_string()
        };
        self.hotkey_inputs[slot].update(cx, |state, cx| {
            state.set_value(text, window, cx);
        });
    }

    fn handle_hotkey_key(
        &mut self,
        slot: usize,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recording != Some(slot) {
            return;
        }
        let key = event.keystroke.key.to_ascii_lowercase();
        if key == "escape" {
            self.recording = None;
            self.status = t(self.lang, "cancel").into();
            cx.notify();
            cx.stop_propagation();
            return;
        }

        let hotkey = if key == "backspace" || key == "delete" {
            Some(Hotkey::default())
        } else {
            key_to_vk(&key).map(|vk| {
                let modifiers = event.keystroke.modifiers;
                let mut mask = 0;
                if modifiers.control {
                    mask |= MOD_CONTROL;
                }
                if modifiers.alt {
                    mask |= MOD_ALT;
                }
                if modifiers.shift {
                    mask |= MOD_SHIFT;
                }
                if modifiers.platform {
                    mask |= MOD_WIN;
                }
                Hotkey { modifiers: mask, vk }
            })
        };
        let Some(hotkey) = hotkey else {
            return;
        };

        self.set_hotkey(slot, hotkey);
        self.recording = None;
        self.sync_hotkey_input(slot, window, cx);
        self.mark_dirty(cx);
        let text = if hotkey.is_empty() {
            t(self.lang, "hotkey_clear").to_string()
        } else {
            hotkey.to_string()
        };
        self.status = format!("{}: {text}", t(self.lang, "hotkey")).into();
        cx.notify();
        cx.stop_propagation();
    }

    fn refresh_presets(&mut self) {
        self.presets = self.store.lock().preset_names();
    }

    fn refresh_monitors(&mut self) {
        let stale = self
            .monitors_at
            .map_or(true, |at| at.elapsed() > Duration::from_secs(3));
        if stale {
            self.monitors = crate::system::monitor::monitors();
            self.monitors_at = Some(Instant::now());
        }
    }
}

impl Drop for SettingsApp {
    fn drop(&mut self) {
        let mut store = self.store.lock();
        if self.save_on_drop {
            let _ = store.save_preset(&self.preset_name, &self.draft);
        }
        store.app.language = self.lang.code().to_string();
        store.app.glass.level = self.glass_level;
        store.app.glass.mode = self.glass.to_mode();
        let _ = store.save_app();
    }
}

// ===========================================================================
// 渲染
// ===========================================================================

impl Render for SettingsApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = self.palette();
        v_flex()
            .size_full()
            .bg(pal.root)
            .text_color(pal.text)
            .font_family(FONT_SANS)
            .child(self.render_title_bar(&pal, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(self.render_sidebar(&pal, cx))
                    .child(self.render_main(&pal, cx)),
            )
            .child(self.render_bottom_bar(&pal, cx))
    }
}

impl SettingsApp {
    fn render_title_bar(&self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = crate::ipc::find_backend().is_some();
        let lang_label = match self.lang {
            Lang::Zh => "中 / EN",
            Lang::En => "EN / 中",
        };

        TitleBar::new()
            .bg(pal.title_bar)
            .border_color(pal.border)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .pl(px(2.))
                    .child(div().w(px(9.)).h(px(9.)).rounded_full().bg(pal.accent))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.text)
                            .child("ACAJA"),
                    )
                    .child(div().text_size(px(10.5)).text_color(pal.dim).child("STUDIO")),
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
                            .bg(pal.control)
                            .border_1()
                            .border_color(pal.border)
                            .child(
                                div()
                                    .w(px(6.))
                                    .h(px(6.))
                                    .rounded_full()
                                    .bg(if connected { pal.success } else { pal.warn }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(pal.muted)
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
                            .hover(|style| style.bg(pal.card_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let next = match this.lang {
                                    Lang::Zh => Lang::En,
                                    Lang::En => Lang::Zh,
                                };
                                this.set_lang(next, cx);
                            }))
                            .child(div().text_size(px(10.)).text_color(pal.muted).child(lang_label)),
                    ),
            )
    }

    fn render_sidebar(&self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut nav = v_flex().gap(px(3.));
        for section in Section::ALL {
            nav = nav.child(self.nav_item(section, pal, cx));
        }

        v_flex()
            .w(px(228.))
            .h_full()
            .p(px(14.))
            .gap(px(3.))
            .bg(pal.sidebar)
            .border_r_1()
            .border_color(pal.border)
            .child(self.sidebar_brand(pal))
            .child(div().h(px(12.)))
            .child(nav)
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap(px(3.))
                    .p(px(11.))
                    .rounded(px(12.))
                    .bg(pal.control)
                    .border_1()
                    .border_color(pal.border)
                    .child(
                        div()
                            .text_size(px(9.5))
                            .text_color(pal.accent)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("GPUI UI"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(pal.dim)
                            .child(format!("v{} · stable", crate::VERSION)),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(pal.dim)
                            .child("Direct2D overlay"),
                    ),
            )
    }

    fn sidebar_brand(&self, pal: &Palette) -> impl IntoElement {
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
                    .bg(pal.accent_soft)
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
                            .text_color(pal.text)
                            .child("ACAJA"),
                    )
                    .child(div().text_size(px(9.5)).text_color(pal.dim).child("CROSSHAIR STUDIO")),
            )
    }

    fn nav_item(&self, section: Section, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
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
                this.bg(pal.accent_soft).border_1().border_color(ca(ACCENT, 0.38))
            })
            .when(!active, |this| this.hover(|style| style.bg(pal.card_hover)))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.section = section;
                this.recording = None;
                cx.notify();
            }))
            .child(nav_glyph(section, active, pal))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.5))
                    .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                    .text_color(if active { pal.text } else { pal.muted })
                    .child(label),
            );

        if active {
            item = item.child(div().w(px(3.)).h(px(14.)).rounded_full().bg(pal.accent));
        }
        item
    }

    fn render_main(&mut self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let section: AnyElement = match self.section {
            Section::Style => self.render_style(pal, cx),
            Section::Dynamic => self.render_dynamic(pal, cx),
            Section::Position => self.render_position(pal, cx),
            Section::Gamepad => self.render_gamepad(pal, cx),
            Section::Hotkey => self.render_hotkey(pal, cx),
            Section::Image => self.render_image(pal, cx),
            Section::Presets => self.render_presets(pal, cx),
            Section::System => self.render_system(pal, cx),
        };

        v_flex().flex_1().min_w(px(0.)).h_full().child(
            div()
                .id("main-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(
                    v_flex()
                        .p(px(28.))
                        .gap(px(20.))
                        .child(self.render_header(pal))
                        .child(section),
                ),
        )
    }

    fn render_header(&self, pal: &Palette) -> impl IntoElement {
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
                            .text_color(pal.text)
                            .child(t(self.lang, self.section.title_key())),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(pal.muted)
                            .child(self.section.subtitle(self.lang)),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(7.))
                    .h(px(30.))
                    .px(px(12.))
                    .rounded_full()
                    .bg(pal.card)
                    .border_1()
                    .border_color(pal.border)
                    .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(pal.accent2))
                    .child(div().text_size(px(9.5)).text_color(pal.dim).child("PRESET"))
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.text)
                            .child(self.preset_name.clone()),
                    ),
            )
    }

    // ---- 形状样式 ---------------------------------------------------------

    fn render_style(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap(px(20.))
            .child(
                h_flex()
                    .gap(px(20.))
                    .items_stretch()
                    .child(self.render_preview_card(pal))
                    .child(self.render_style_controls(pal, cx)),
            )
            .child(self.render_shape_gallery(pal, cx))
            .child(self.render_templates(pal, cx))
            .into_any_element()
    }

    fn render_preview_card(&self, pal: &Palette) -> impl IntoElement {
        let preview = self.draft.clone();
        let shape_label = shape_name(self.lang, self.draft.shape);
        let size_label = format!("{:.0} px", self.draft.size);
        let thickness_label = format!("{:.1} px", self.draft.thickness);
        let opacity_label = format!("{:.0}%", self.draft.opacity * 100.0);

        v_flex()
            .flex_1()
            .min_w(px(0.))
            .h(px(432.))
            .rounded(px(24.))
            .overflow_hidden()
            .bg(c(0x0A0A0FFF))
            .border_1()
            .border_color(pal.border)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        canvas(
                            move |bounds, _, _| bounds,
                            move |_, bounds, window, _| {
                                paint_preview(window, bounds, &preview, 0.0);
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
                            .child(div().text_size(px(9.5)).text_color(pal.muted).child("CENTER · 1:1")),
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
                    .border_color(pal.border)
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(pal.text)
                                    .child(shape_label),
                            )
                            .child(div().w(px(1.)).h(px(12.)).bg(pal.border))
                            .child(div().text_size(px(10.5)).text_color(pal.muted).child(size_label))
                            .child(div().text_size(px(10.5)).text_color(pal.dim).child("·"))
                            .child(div().text_size(px(10.5)).text_color(pal.muted).child(thickness_label))
                            .child(div().text_size(px(10.5)).text_color(pal.dim).child("·"))
                            .child(div().text_size(px(10.5)).text_color(pal.muted).child(opacity_label)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(6.))
                            .child(div().w(px(5.)).h(px(5.)).rounded_full().bg(pal.success))
                            .child(div().text_size(px(9.5)).text_color(pal.dim).child("LIVE")),
                    ),
            )
    }

    fn render_style_controls(&self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut controls = v_flex()
            .w(px(392.))
            .flex_shrink_0()
            .gap(px(14.))
            .p(px(22.))
            .rounded(px(24.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(section_label(t(self.lang, "shape_style"), pal));

        if self.draft.multicolor {
            for (index, label) in [
                (1, t(self.lang, "color_top")),
                (2, t(self.lang, "color_bottom")),
                (3, t(self.lang, "color_left")),
                (4, t(self.lang, "color_right")),
            ] {
                controls = controls.child(self.color_field(index, label, pal, cx));
            }
        } else {
            controls = controls.child(self.color_field(0, t(self.lang, "main_color"), pal, cx));
            controls = controls.child(self.color_palette(pal, cx));
        }

        controls = controls
            .child(self.slider_row(t(self.lang, "size"), self.draft.size, "px", &self.size_slider, pal))
            .child(self.slider_row(
                t(self.lang, "thickness"),
                self.draft.thickness,
                "px",
                &self.thickness_slider,
                pal,
            ))
            .child(self.slider_row(
                t(self.lang, "opacity"),
                self.draft.opacity * 100.0,
                "%",
                &self.opacity_slider,
                pal,
            ))
            .child(self.slider_row(
                t(self.lang, "rotation"),
                self.draft.rotation,
                "°",
                &self.rotation_slider,
                pal,
            ));

        if matches!(
            self.draft.shape,
            CrossShape::HollowCross
                | CrossShape::HollowSquare
                | CrossShape::HollowCrossDot
                | CrossShape::GapHair
        ) {
            controls = controls.child(self.slider_row(
                t(self.lang, "hollow_gap"),
                self.draft.hollow.gap,
                "",
                &self.gap_slider,
                pal,
            ));
            if matches!(self.draft.shape, CrossShape::HollowCrossDot | CrossShape::GapHair) {
                controls = controls.child(self.slider_row(
                    t(self.lang, "center_dot"),
                    self.draft.hollow.center_dot_size,
                    "",
                    &self.dot_slider,
                    pal,
                ));
            }
        }

        controls = controls
            .child(div().h(px(1.)).bg(pal.border))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .h(px(30.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(pal.muted)
                            .child(t(self.lang, "multicolor")),
                    )
                    .child(
                        Switch::new("multicolor")
                            .checked(self.draft.multicolor)
                            .on_change(cx.listener(|this, value, window, cx| {
                                this.set_multicolor(*value, window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .h(px(30.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(pal.muted)
                            .child(t(self.lang, "outline")),
                    )
                    .child(
                        Switch::new("outline")
                            .checked(self.draft.outline.enabled)
                            .on_change(cx.listener(|this, value, _, cx| {
                                this.set_outline(*value, cx);
                            })),
                    ),
            );

        if self.draft.outline.enabled {
            controls = controls
                .child(self.slider_row(
                    t(self.lang, "outline_thickness"),
                    self.draft.outline.thickness,
                    "px",
                    &self.outline_thickness_slider,
                    pal,
                ))
                .child(self.color_field(5, t(self.lang, "outline_color"), pal, cx))
                .child(self.slider_row(
                    t(self.lang, "outline_opacity"),
                    self.draft.outline.opacity * 100.0,
                    "%",
                    &self.outline_opacity_slider,
                    pal,
                ));
        }

        controls
    }

    fn render_shape_gallery(&self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = v_flex().gap(px(8.));
        for chunk in CrossShape::ALL.chunks(10) {
            let mut row = h_flex().gap(px(8.));
            for shape in chunk {
                row = row.child(self.shape_tile(*shape, pal, cx));
            }
            rows = rows.child(row);
        }

        v_flex()
            .gap(px(10.))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(section_label(t(self.lang, "shape"), pal))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(pal.dim)
                            .child(format!("{} SHAPES", CrossShape::ALL.len())),
                    ),
            )
            .child(rows)
    }

    fn shape_tile(&self, shape: CrossShape, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.draft.shape == shape;
        let color = if selected {
            hex_color(&self.draft.color, 1.0)
        } else {
            pal.muted
        };
        let label = shape_name(self.lang, shape);

        div()
            .id(("shape", shape as usize))
            .w(px(84.))
            .h(px(84.))
            .rounded(px(14.))
            .border_1()
            .border_color(if selected { pal.accent } else { pal.border })
            .bg(if selected { pal.accent_soft } else { pal.control })
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .cursor_pointer()
            .hover(|style| style.bg(pal.card_hover).border_color(pal.border))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.draft.shape = shape;
                this.mark_dirty(cx);
            }))
            .child(shape_thumb(shape, color))
            .child(
                div()
                    .text_size(px(9.5))
                    .text_color(if selected { pal.text } else { pal.dim })
                    .child(label),
            )
    }

    fn render_templates(&self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = v_flex().gap(px(10.));
        for chunk in TEMPLATES.chunks(4) {
            let mut row = h_flex().gap(px(10.));
            for (index, (key, _)) in chunk.iter().enumerate() {
                let abs_index = TEMPLATES.iter().position(|(k, _)| k == key).unwrap_or(index);
                let preview = self.tpl_previews[abs_index].clone();
                let label = t(self.lang, key);
                row = row.child(
                    div()
                        .id(("template", abs_index))
                        .flex_1()
                        .h(px(126.))
                        .rounded(px(16.))
                        .bg(pal.card)
                        .border_1()
                        .border_color(pal.border)
                        .p(px(8.))
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .cursor_pointer()
                        .hover(|style| style.bg(pal.card_hover).border_color(pal.border))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            (TEMPLATES[abs_index].1)(&mut this.draft);
                            this.sync_inputs(window, cx);
                            this.mark_dirty(cx);
                            this.status = t(this.lang, "template_applied").into();
                            cx.notify();
                        }))
                        .child(
                            div()
                                .flex_1()
                                .rounded(px(10.))
                                .overflow_hidden()
                                .bg(c(0x0A0A0FFF))
                                .child(
                                    canvas(
                                        move |bounds, _, _| bounds,
                                        move |_, bounds, window, _| {
                                            paint_preview(window, bounds, &preview, 0.0);
                                        },
                                    )
                                    .size_full(),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(pal.muted)
                                .child(label),
                        ),
                );
            }
            rows = rows.child(row);
        }

        v_flex()
            .gap(px(8.))
            .child(section_label(t(self.lang, "templates"), pal))
            .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "templates_hint")))
            .child(rows)
    }

    fn color_field(
        &self,
        index: usize,
        label: &'static str,
        pal: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hex = color_value(&self.draft, index);
        h_flex()
            .items_center()
            .gap(px(8.))
            .child(div().w(px(70.)).text_size(px(11.)).text_color(pal.muted).child(label))
            .child(Input::new(&self.color_inputs[index]).w_full())
            .child(
                div()
                    .w(px(24.))
                    .h(px(24.))
                    .rounded(px(6.))
                    .bg(hex_color(&hex, 1.0))
                    .border_1()
                    .border_color(pal.border),
            )
            .child(
                div()
                    .id(("color-reset", index))
                    .w(px(20.))
                    .h(px(20.))
                    .rounded(px(5.))
                    .bg(pal.control)
                    .border_1()
                    .border_color(pal.border)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(pal.card_hover))
                    .child(div().text_size(px(9.5)).text_color(pal.dim).child("↺"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let default = if index == 5 { "#000000" } else { "#FF0000" };
                        this.set_color_value(index, default, window, cx);
                    })),
            )
    }

    fn color_palette(&self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = h_flex().gap(px(6.)).items_center();
        for (index, hex) in COLOR_PRESETS.iter().enumerate() {
            let value = *hex;
            row = row.child(
                div()
                    .id(("swatch", index))
                    .w(px(22.))
                    .h(px(22.))
                    .rounded_full()
                    .bg(hex_color(value, 1.0))
                    .border_2()
                    .border_color(if self.draft.color.eq_ignore_ascii_case(value) {
                        pal.text
                    } else {
                        ca(0xFFFFFF, 0.10)
                    })
                    .cursor_pointer()
                    .hover(|style| style.border_color(ca(0xFFFFFF, 0.40)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_color_value(0, value, window, cx);
                    })),
            );
        }
        row
    }

    fn slider_row(
        &self,
        label: &'static str,
        value: f32,
        unit: &'static str,
        slider: &Entity<SliderState>,
        pal: &Palette,
    ) -> impl IntoElement {
        v_flex()
            .gap(px(7.))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(11.5)).text_color(pal.muted).child(label))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.text)
                            .child(format!("{value:.1} {unit}")),
                    ),
            )
            .child(Slider::new(slider).w_full())
    }

    // ---- 动态准星 ---------------------------------------------------------

    fn render_dynamic(&mut self, pal: &Palette, _cx: &mut Context<Self>) -> AnyElement {
        let preview = self.draft.clone();
        let expand = self.draft.dynamic.fire_expand_px;
        v_flex()
            .child(
                h_flex()
                    .gap(px(20.))
                    .items_stretch()
                    .child(
                        v_flex()
                            .flex_1()
                            .gap(px(14.))
                            .p(px(22.))
                            .rounded(px(22.))
                            .bg(pal.card)
                            .border_1()
                            .border_color(pal.border)
                            .child(section_label(t(self.lang, "dynamic"), pal))
                            .child(self.slider_row(
                                t(self.lang, "fire_expand"),
                                self.draft.dynamic.fire_expand_px,
                                "px",
                                &self.dynamic_expand_slider,
                                pal,
                            ))
                            .child(self.slider_row(
                                t(self.lang, "recover_ms"),
                                self.draft.dynamic.recover_ms as f32,
                                "ms",
                                &self.dynamic_recover_slider,
                                pal,
                            ))
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .h(px(30.))
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(pal.muted)
                                            .child(t(self.lang, "recoil_indicator")),
                                    )
                                    .child(
                                        Switch::new("recoil")
                                            .checked(self.draft.dynamic.recoil_indicator)
                                            .on_change(_cx.listener(|this, value, _, cx| {
                                                this.draft.dynamic.recoil_indicator = *value;
                                                this.mark_dirty(cx);
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .w(px(400.))
                            .flex_shrink_0()
                            .rounded(px(22.))
                            .overflow_hidden()
                            .bg(c(0x0A0A0FFF))
                            .border_1()
                            .border_color(pal.border)
                            .child(
                                div()
                                    .h(px(260.))
                                    .relative()
                                    .child(
                                        canvas(
                                            move |bounds, _, _| bounds,
                                            move |_, bounds, window, _| {
                                                paint_preview(window, bounds, &preview, expand);
                                            },
                                        )
                                        .size_full(),
                                    )
                                    .child(top_chip("FIRE SPREAD", ACCENT_2)),
                            )
                            .child(
                                h_flex()
                                    .h(px(44.))
                                    .px(px(16.))
                                    .items_center()
                                    .justify_between()
                                    .bg(c(0x0D0D12FF))
                                    .border_t_1()
                                    .border_color(pal.border)
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(pal.muted)
                                            .child(t(self.lang, "fire_expand")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(pal.accent2)
                                            .child(format!("+{expand:.0}px")),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    // ---- 位置 -------------------------------------------------------------

    fn render_position(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        self.refresh_monitors();
        let monitors = self.monitors.clone();

        let mut monitor_rows = v_flex().gap(px(6.));
        monitor_rows = monitor_rows.child(self.monitor_row(
            -1,
            t(self.lang, "monitor_follow").to_string(),
            pal,
            cx,
        ));
        for (index, monitor) in monitors.iter().enumerate() {
            let width = monitor.rect.2 - monitor.rect.0;
            let height = monitor.rect.3 - monitor.rect.1;
            let mut label = format!("{} {} · {}×{}", t(self.lang, "monitor"), index + 1, width, height);
            if monitor.primary {
                label.push_str(" · ");
                label.push_str(t(self.lang, "monitor_primary"));
            }
            monitor_rows = monitor_rows.child(self.monitor_row(index as i32, label, pal, cx));
        }

        let left = v_flex()
            .flex_1()
            .gap(px(10.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(section_label(t(self.lang, "monitors_title"), pal))
                    .child(
                        Button::new("monitor-refresh")
                            .xsmall()
                            .ghost()
                            .label(t(self.lang, "monitor_refresh"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.monitors = crate::system::monitor::monitors();
                                this.monitors_at = Some(Instant::now());
                                cx.notify();
                            })),
                    ),
            )
            .child(monitor_rows);

        let right = v_flex()
            .w(px(390.))
            .flex_shrink_0()
            .gap(px(10.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(section_label(t(self.lang, "position"), pal))
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(div().w(px(22.)).text_size(px(11.5)).text_color(pal.muted).child("X"))
                    .child(Input::new(&self.pos_x_input).w_full())
                    .child(div().w(px(22.)).text_size(px(11.5)).text_color(pal.muted).child("Y"))
                    .child(Input::new(&self.pos_y_input).w_full()),
            )
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(
                        Button::new("center-monitor")
                            .small()
                            .ghost()
                            .label(t(self.lang, "center_on_monitor"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                let selected = this.draft.position.monitor;
                                let target = if selected < 0 {
                                    crate::system::monitor::primary()
                                        .map(|m| m.work_center())
                                        .unwrap_or((0, 0))
                                } else {
                                    this.monitors
                                        .get(selected as usize)
                                        .or_else(|| this.monitors.first())
                                        .map(|m| m.work_center())
                                        .unwrap_or((0, 0))
                                };
                                this.draft.position.x = PosVal::Px(target.0 as f32);
                                this.draft.position.y = PosVal::Px(target.1 as f32);
                                this.sync_inputs(window, cx);
                                this.mark_dirty(cx);
                            })),
                    )
                    .child(
                        Button::new("center-screen")
                            .small()
                            .ghost()
                            .label(t(self.lang, "center_btn"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.draft.position.x = PosVal::Center;
                                this.draft.position.y = PosVal::Center;
                                this.sync_inputs(window, cx);
                                this.mark_dirty(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .h(px(30.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(pal.muted)
                            .child(t(self.lang, "snap_to_window")),
                    )
                    .child(
                        Switch::new("snap")
                            .checked(self.draft.snap_to_window)
                            .on_change(cx.listener(|this, value, _, cx| {
                                this.draft.snap_to_window = *value;
                                this.mark_dirty(cx);
                            })),
                    ),
            )
            .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "snap_note")));

        h_flex()
            .gap(px(20.))
            .items_stretch()
            .child(left)
            .child(right)
            .into_any_element()
    }

    fn monitor_row(
        &self,
        index: i32,
        label: String,
        pal: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.draft.position.monitor == index;
        div()
            .id(if index < 0 { "monitor-follow".to_string() } else { format!("monitor-{index}") })
            .h(px(38.))
            .px(px(12.))
            .rounded(px(10.))
            .flex()
            .items_center()
            .gap(px(9.))
            .cursor_pointer()
            .bg(if selected { pal.accent_soft } else { pal.control })
            .border_1()
            .border_color(if selected { ca(ACCENT, 0.40) } else { pal.border })
            .hover(|style| style.bg(pal.card_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.draft.position.monitor = index;
                this.mark_dirty(cx);
            }))
            .child(
                div()
                    .w(px(7.))
                    .h(px(7.))
                    .rounded_full()
                    .bg(if selected { pal.accent } else { pal.dim }),
            )
            .child(div().text_size(px(11.5)).text_color(if selected { pal.text } else { pal.muted }).child(label))
    }

    // ---- 手柄 -------------------------------------------------------------

    fn render_gamepad(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let ads_options = [
            (AdsMode::Off, ads_mode_name(self.lang, AdsMode::Off)),
            (AdsMode::HoldHide, ads_mode_name(self.lang, AdsMode::HoldHide)),
            (AdsMode::Toggle, ads_mode_name(self.lang, AdsMode::Toggle)),
            (AdsMode::HoldShow, ads_mode_name(self.lang, AdsMode::HoldShow)),
        ];
        let button_options = [
            (AdsButton::LeftTrigger, t(self.lang, "ads_left_trigger")),
            (AdsButton::RightTrigger, t(self.lang, "ads_right_trigger")),
            (AdsButton::LeftBumper, t(self.lang, "ads_left_bumper")),
            (AdsButton::RightBumper, t(self.lang, "ads_right_bumper")),
        ];

        let left = v_flex()
            .flex_1()
            .gap(px(14.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(section_label(t(self.lang, "gamepad"), pal))
            .child(choice_row(
                t(self.lang, "ads_mode"),
                &ads_options,
                self.draft.gamepad.ads_mode,
                pal,
                cx,
                |this, value| this.draft.gamepad.ads_mode = value,
            ))
            .child(choice_row(
                t(self.lang, "ads_button"),
                &button_options,
                self.draft.gamepad.ads_button,
                pal,
                cx,
                |this, value| this.draft.gamepad.ads_button = value,
            ))
            .child(self.slider_row(
                t(self.lang, "trigger_threshold"),
                self.draft.gamepad.trigger_threshold as f32,
                "",
                &self.threshold_slider,
                pal,
            ))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .h(px(30.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(pal.muted)
                            .child(t(self.lang, "gamepad_fire_expand")),
                    )
                    .child(
                        Switch::new("gamepad-fire")
                            .checked(self.draft.gamepad.fire_expand)
                            .on_change(cx.listener(|this, value, _, cx| {
                                this.draft.gamepad.fire_expand = *value;
                                this.mark_dirty(cx);
                            })),
                    ),
            );

        let gamepad_pal = *pal;
        let right = v_flex()
            .w(px(360.))
            .flex_shrink_0()
            .gap(px(12.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(
                div()
                    .h(px(96.))
                    .rounded(px(14.))
                    .bg(pal.control)
                    .border_1()
                    .border_color(pal.border)
                    .child(
                        canvas(
                            move |bounds, _, _| bounds,
                            move |_, bounds, window, _| paint_gamepad(window, bounds, &gamepad_pal),
                        )
                        .size_full(),
                    ),
            )
            .child(section_label("XInput", pal))
            .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "gamepad_note")));

        h_flex()
            .gap(px(20.))
            .items_stretch()
            .child(left)
            .child(right)
            .into_any_element()
    }

    // ---- 快捷键 -----------------------------------------------------------

    fn render_hotkey(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let left = v_flex()
            .w(px(560.))
            .flex_shrink_0()
            .gap(px(12.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(section_label(t(self.lang, "hotkey"), pal))
            .child(self.hotkey_field(0, t(self.lang, "hotkey_toggle"), pal, cx))
            .child(self.hotkey_field(1, t(self.lang, "hotkey_next"), pal, cx))
            .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "hotkey_note")));

        let mut right = v_flex()
            .flex_1()
            .gap(px(12.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(section_label(t(self.lang, "right_click"), pal))
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .h(px(30.))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(pal.muted)
                            .child(t(self.lang, "right_click")),
                    )
                    .child(
                        Switch::new("right-click")
                            .checked(self.draft.right_click_toggle)
                            .on_change(cx.listener(|this, value, _, cx| {
                                this.draft.right_click_toggle = *value;
                                this.mark_dirty(cx);
                            })),
                    ),
            );

        if self.draft.right_click_toggle {
            let options = [
                (RightClickMode::Click, t(self.lang, "rc_click")),
                (RightClickMode::HoldShow, t(self.lang, "rc_hold_show")),
                (RightClickMode::HoldHide, t(self.lang, "rc_hold_hide")),
            ];
            right = right.child(choice_row(
                t(self.lang, "right_click_mode"),
                &options,
                self.draft.right_click_mode,
                pal,
                cx,
                |this, value| this.draft.right_click_mode = value,
            ));
        }

        h_flex()
            .gap(px(20.))
            .items_stretch()
            .child(left)
            .child(right)
            .into_any_element()
    }

    fn hotkey_field(
        &self,
        slot: usize,
        label: &'static str,
        pal: &Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let recording = self.recording == Some(slot);
        let hotkey = if slot == 0 {
            self.draft.hotkey_toggle
        } else {
            self.draft.hotkey_next_profile
        };
        let text = if recording {
            t(self.lang, "hotkey_recording").to_string()
        } else if hotkey.is_empty() {
            t(self.lang, "hotkey_record").to_string()
        } else {
            hotkey.to_string()
        };

        v_flex()
            .gap(px(6.))
            .child(div().text_size(px(11.)).text_color(pal.muted).child(label))
            .child(
                h_flex()
                    .gap(px(6.))
                    .child(
                        div()
                            .id(("hotkey-box", slot))
                            .flex_1()
                            .h(px(34.))
                            .px(px(12.))
                            .rounded(px(9.))
                            .bg(pal.control)
                            .border_1()
                            .border_color(if recording { pal.accent } else { pal.border })
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .track_focus(&self.hotkey_focus[slot])
                            .on_key_down(cx.listener(move |this, event, window, cx| {
                                this.handle_hotkey_key(slot, event, window, cx);
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.recording = Some(slot);
                                let handle = this.hotkey_focus[slot].clone();
                                window.focus(&handle, cx);
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(if hotkey.is_empty() && !recording { pal.dim } else { pal.text })
                                    .truncate()
                                    .child(text),
                            ),
                    )
                    .child(
                        Button::new(("hotkey-clear", slot))
                            .xsmall()
                            .ghost()
                            .label(t(self.lang, "hotkey_clear"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_hotkey(slot, Hotkey::default());
                                this.sync_hotkey_input(slot, window, cx);
                                this.recording = None;
                                this.mark_dirty(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(70.))
                            .text_size(px(10.5))
                            .text_color(pal.dim)
                            .child(t(self.lang, "hotkey_manual")),
                    )
                    .child(Input::new(&self.hotkey_inputs[slot]).w_full()),
            )
    }

    // ---- 图片 -------------------------------------------------------------

    fn render_image(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let path = self.draft.image.path.clone();
        let has_file = !path.is_empty() && Path::new(&path).is_file();
        let thumb: AnyElement = if has_file {
            img(std::path::PathBuf::from(path.clone()))
                .w_full()
                .h_full()
                .into_any_element()
        } else {
            div()
                .text_size(px(11.))
                .text_color(pal.dim)
                .child(t(self.lang, if path.is_empty() { "image_preview" } else { "image_unreadable" }))
                .into_any_element()
        };
        let display = if path.is_empty() {
            t(self.lang, "image_none").to_string()
        } else {
            Path::new(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&path)
                .to_string()
        };

        v_flex()
            .child(
                h_flex()
                    .gap(px(20.))
                    .items_stretch()
                    .child(
                        v_flex()
                            .w(px(250.))
                            .flex_shrink_0()
                            .rounded(px(22.))
                            .overflow_hidden()
                            .bg(pal.control)
                            .border_1()
                            .border_color(pal.border)
                            .child(
                                div()
                                    .h(px(180.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(thumb),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap(px(12.))
                            .p(px(22.))
                            .rounded(px(22.))
                            .bg(pal.card)
                            .border_1()
                            .border_color(pal.border)
                            .child(section_label(t(self.lang, "custom_image"), pal))
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(pal.dim)
                                    .child(t(self.lang, "image_path")),
                            )
                            .child(
                                div()
                                    .h(px(32.))
                                    .px(px(10.))
                                    .rounded(px(9.))
                                    .bg(pal.control)
                                    .border_1()
                                    .border_color(pal.border)
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(if path.is_empty() { pal.dim } else { pal.text })
                                            .truncate()
                                            .child(display),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .child(
                                        Button::new("image-pick")
                                            .small()
                                            .ghost()
                                            .label(t(self.lang, "image_choose"))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Some(path) = crate::system::filedialog::pick_image() {
                                                    this.draft.image.path = path.to_string_lossy().into_owned();
                                                    this.mark_dirty(cx);
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("image-clear")
                                            .small()
                                            .ghost()
                                            .label(t(self.lang, "clear"))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.draft.image.path.clear();
                                                this.mark_dirty(cx);
                                            })),
                                    ),
                            )
                            .child(self.slider_row(
                                t(self.lang, "image_scale"),
                                self.draft.image.scale,
                                "x",
                                &self.image_scale_slider,
                                pal,
                            ))
                            .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "image_none"))),
                    ),
            )
            .into_any_element()
    }
}

// ===========================================================================
// 预设 / 绑定 / 系统（单独 impl 以控制单函数长度）
// ===========================================================================

impl SettingsApp {
    fn render_presets(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        self.refresh_presets();
        let presets = self.presets.clone();

        let mut list = v_flex().gap(px(4.));
        for (index, name) in presets.iter().enumerate() {
            list = list.child(self.render_preset_row(name, index, pal, cx));
            if self.rename_from.as_deref() == Some(name.as_str()) {
                list = list.child(self.render_rename_row(name, pal, cx));
            }
        }

        let top = h_flex()
            .items_center()
            .gap(px(8.))
            .child(
                Button::new("preset-save")
                    .small()
                    .ghost()
                    .label(t(self.lang, "save"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_current(cx))),
            )
            .child(
                Button::new("preset-import")
                    .small()
                    .ghost()
                    .label(t(self.lang, "import"))
                    .on_click(cx.listener(|this, _, window, cx| this.import_preset(window, cx))),
            )
            .child(Input::new(&self.new_preset_input).w(px(190.)))
            .child(
                Button::new("preset-create")
                    .small()
                    .primary()
                    .label(t(self.lang, "create"))
                    .on_click(cx.listener(|this, _, window, cx| this.create_preset(window, cx))),
            );

        let preset_card = v_flex()
            .gap(px(12.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(section_label(t(self.lang, "presets"), pal))
            .child(top)
            .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "new_preset")))
            .child(list);

        v_flex()
            .gap(px(20.))
            .child(preset_card)
            .child(self.render_bindings(pal, cx))
            .into_any_element()
    }

    fn render_preset_row(
        &self,
        name: &str,
        index: usize,
        pal: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = name == self.preset_name;
        let confirming = self.delete_confirm.as_deref() == Some(name);
        let name_for_name_area = name.to_string();
        let name_for_use = name.to_string();
        let name_for_dup = name.to_string();
        let name_for_ren = name.to_string();
        let name_for_exp = name.to_string();
        let name_for_del = name.to_string();

        let mut row = h_flex()
            .items_center()
            .gap(px(8.))
            .h(px(40.))
            .px(px(10.))
            .rounded(px(10.))
            .when(active, |this| {
                this.bg(pal.accent_soft).border_1().border_color(ca(ACCENT, 0.35))
            })
            .when(!active, |this| this.hover(|style| style.bg(pal.card_hover)))
            .child(
                div()
                    .w(px(7.))
                    .h(px(7.))
                    .rounded_full()
                    .bg(if active { pal.success } else { pal.dim }),
            )
            .child(
                div()
                    .id(("preset-name", index))
                    .flex_1()
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .font_weight(if active { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                    .text_color(if active { pal.text } else { pal.muted })
                    .child(name.to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !active {
                            this.apply_preset(&name_for_name_area, window, cx);
                        }
                    })),
            );

        if confirming {
            row = row
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(pal.warn)
                        .child(format!("{} {}?", t(self.lang, "delete_confirm"), name)),
                )
                .child(
                    Button::new(("preset-del-cancel", index))
                        .xsmall()
                        .ghost()
                        .label(t(self.lang, "cancel"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.delete_confirm = None;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("preset-del-confirm", index))
                        .xsmall()
                        .danger()
                        .label(t(self.lang, "confirm"))
                        .on_click(cx.listener(|this, _, window, cx| this.confirm_delete(window, cx))),
                );
        } else {
            if !active {
                row = row.child(
                    Button::new(("preset-use", index))
                        .xsmall()
                        .ghost()
                        .label(t(self.lang, "activate"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.apply_preset(&name_for_use, window, cx);
                        })),
                );
            }
            row = row
                .child(
                    Button::new(("preset-dup", index))
                        .xsmall()
                        .ghost()
                        .label(t(self.lang, "duplicate"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.duplicate_preset(&name_for_dup, window, cx);
                        })),
                )
                .child(
                    Button::new(("preset-ren", index))
                        .xsmall()
                        .ghost()
                        .label(t(self.lang, "rename"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.start_rename(&name_for_ren, window, cx);
                        })),
                )
                .child(
                    Button::new(("preset-exp", index))
                        .xsmall()
                        .ghost()
                        .label(t(self.lang, "export"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.export_preset(&name_for_exp, cx);
                        })),
                )
                .child(
                    Button::new(("preset-del", index))
                        .xsmall()
                        .ghost()
                        .label(t(self.lang, "btn_delete"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.request_delete(&name_for_del, cx);
                        })),
                );
        }

        row.into_any_element()
    }

    fn render_rename_row(&self, name: &str, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let current = self.rename_input.read(cx).value().to_string();
        let trimmed = current.trim();
        let taken = self.presets.iter().any(|p| p == trimmed) && trimmed != name;
        let message = if let Err(err) = validate_preset_name(&current) {
            err.to_string()
        } else if trimmed == name {
            t(self.lang, "rename").to_string()
        } else if taken {
            t(self.lang, "rename_failed").to_string()
        } else {
            String::new()
        };
        let color = if message.is_empty() { pal.dim } else { pal.warn };

        v_flex()
            .gap(px(4.))
            .pl(px(28.))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .child(Input::new(&self.rename_input).w(px(240.)))
                    .child(
                        Button::new("rename-confirm")
                            .xsmall()
                            .primary()
                            .label(t(self.lang, "confirm"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_rename(window, cx);
                            })),
                    )
                    .child(
                        Button::new("rename-cancel")
                            .xsmall()
                            .ghost()
                            .label(t(self.lang, "cancel"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.rename_from = None;
                                this.rename_confirm = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().text_size(px(9.5)).text_color(color).child(message))
            .into_any_element()
    }

    fn render_bindings(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let bindings = self.store.lock().app.game_bindings.clone();
        let presets = self.presets.clone();

        let mut rows = v_flex().gap(px(6.));
        if bindings.is_empty() {
            rows = rows.child(div().text_size(px(11.)).text_color(pal.dim).child(t(self.lang, "binding_none")));
        }
        for (index, binding) in bindings.iter().enumerate() {
            rows = rows.child(
                h_flex()
                    .items_center()
                    .gap(px(8.))
                    .h(px(34.))
                    .px(px(10.))
                    .rounded(px(9.))
                    .bg(pal.control)
                    .border_1()
                    .border_color(pal.border)
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .text_color(pal.text)
                            .child(binding.exe.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.accent2)
                            .child(binding.preset.clone()),
                    )
                    .child(
                        Button::new(("binding-remove", index))
                            .xsmall()
                            .ghost()
                            .label(t(self.lang, "binding_remove"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_binding(index, cx);
                            })),
                    ),
            );
        }

        let mut chips = v_flex().gap(px(6.));
        for chunk in presets.chunks(3) {
            let mut row = h_flex().gap(px(6.));
            for name in chunk {
                let selected = self.binding_preset == *name;
                let value = name.clone();
                row = row.child(
                    div()
                        .id(format!("binding-preset-{name}"))
                        .h(px(26.))
                        .px(px(9.))
                        .rounded(px(7.))
                        .bg(if selected { pal.accent_soft } else { pal.control })
                        .border_1()
                        .border_color(if selected { ca(ACCENT, 0.40) } else { pal.border })
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .hover(|style| style.bg(pal.card_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.binding_preset = value.clone();
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(if selected { pal.text } else { pal.muted })
                                .child(name.clone()),
                        ),
                );
            }
            chips = chips.child(row);
        }

        v_flex()
            .gap(px(12.))
            .p(px(22.))
            .rounded(px(22.))
            .bg(pal.card)
            .border_1()
            .border_color(pal.border)
            .child(section_label(t(self.lang, "bindings"), pal))
            .child(rows)
            .child(Input::new(&self.binding_exe_input).w_full())
            .child(
                div()
                    .id("binding-preset-scroll")
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .child(chips),
            )
            .child(
                Button::new("binding-add")
                    .small()
                    .primary()
                    .label(t(self.lang, "binding_add"))
                    .on_click(cx.listener(|this, _, window, cx| this.add_binding(window, cx))),
            )
            .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "binding_note")))
            .into_any_element()
    }

    fn render_system(&mut self, pal: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let autostart = self.store.lock().app.autostart;

        let mut glass_tiles = h_flex().gap(px(10.));
        for choice in GpuiGlass::ALL {
            let selected = self.glass == choice;
            glass_tiles = glass_tiles.child(
                div()
                    .id(("glass", choice as usize))
                    .flex_1()
                    .p(px(14.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(if selected { pal.accent } else { pal.border })
                    .bg(if selected { pal.accent_soft } else { pal.control })
                    .cursor_pointer()
                    .hover(|style| style.bg(pal.card_hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.glass = choice;
                        window.set_background_appearance(choice.appearance());
                        {
                            let mut store = this.store.lock();
                            store.app.glass.mode = choice.to_mode();
                            let _ = store.save_app();
                        }
                        this.status = format!("{} · {}", choice.label(), choice.note()).into();
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(pal.text)
                            .child(choice.label()),
                    )
                    .child(div().text_size(px(9.5)).text_color(pal.dim).child(choice.note())),
            );
        }

        v_flex()
            .gap(px(18.))
            .child(
                v_flex()
                    .gap(px(12.))
                    .p(px(22.))
                    .rounded(px(22.))
                    .bg(pal.card)
                    .border_1()
                    .border_color(pal.border)
                    .child(section_label(t(self.lang, "system"), pal))
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .h(px(34.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(pal.text)
                                    .child(t(self.lang, "autostart")),
                            )
                            .child(
                                Switch::new("autostart")
                                    .checked(autostart)
                                    .on_change(cx.listener(|this, value, _, cx| {
                                        this.toggle_autostart(*value, cx);
                                    })),
                            ),
                    )
                    .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "autostart_note"))),
            )
            .child(
                v_flex()
                    .gap(px(12.))
                    .p(px(22.))
                    .rounded(px(22.))
                    .bg(pal.card)
                    .border_1()
                    .border_color(pal.border)
                    .child(section_label(t(self.lang, "glass_mode"), pal))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(pal.dim)
                            .child(match self.lang {
                                Lang::Zh => "运行时切换 GPUI 的 WindowBackgroundAppearance。Win10 用 Acrylic；Mica 需要 Win11 22H2+。",
                                Lang::En => "Switch GPUI's WindowBackgroundAppearance at runtime. Use Acrylic on Win10; Mica needs Win11 22H2+.",
                            }),
                    )
                    .child(glass_tiles)
                    .child(self.slider_row(
                        t(self.lang, "glass_level"),
                        self.glass_level,
                        "",
                        &self.glass_level_slider,
                        pal,
                    ))
                    .child(div().text_size(px(10.5)).text_color(pal.dim).child(t(self.lang, "glass_level_hint"))),
            )
            .child(
                v_flex()
                    .gap(px(10.))
                    .p(px(22.))
                    .rounded(px(22.))
                    .bg(pal.card)
                    .border_1()
                    .border_color(pal.border)
                    .child(section_label("ACAJA", pal))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(pal.muted)
                            .child(match self.lang {
                                Lang::Zh => "准星覆盖层仍是 Direct2D；设置界面已迁移到 GPUI Kit。",
                                Lang::En => "The crosshair overlay remains Direct2D; the settings UI now runs on GPUI Kit.",
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(pal.dim)
                            .child(format!("v{} · log: %APPDATA%/ACAJACrosshair/acaja-ui-diag.log", crate::VERSION)),
                    )
                    .child(
                        Button::new("quit-backend")
                            .small()
                            .ghost()
                            .label(t(self.lang, "quit_backend"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(appdata) = crate::appdata_dir() {
                                    let _ = std::fs::write(appdata.join("cmd.json"), "{\"cmd\":\"quit\"}");
                                }
                                this.status = t(this.lang, "quit_backend_sent").into();
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_bottom_bar(&self, pal: &Palette, cx: &mut Context<Self>) -> impl IntoElement {
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
            .bg(ca(0x0A0A0D, 0.95))
            .border_t_1()
            .border_color(pal.border)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(9.))
                    .child(
                        div()
                            .w(px(7.))
                            .h(px(7.))
                            .rounded_full()
                            .bg(if connected { pal.success } else { pal.warn }),
                    )
                    .child(
                        div()
                            .max_w(px(520.))
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(pal.muted)
                            .child(self.status.clone()),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        Button::new("quit-backend-bottom")
                            .ghost()
                            .label(t(self.lang, "quit_backend"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(appdata) = crate::appdata_dir() {
                                    let _ = std::fs::write(appdata.join("cmd.json"), "{\"cmd\":\"quit\"}");
                                }
                                this.status = t(this.lang, "quit_backend_sent").into();
                                cx.notify();
                            })),
                    )
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
// 小部件 / 输入订阅
// ===========================================================================

fn subscribe_text<F>(
    cx: &mut Context<SettingsApp>,
    window: &mut Window,
    input: &Entity<InputState>,
    handler: F,
) -> Subscription
where
    F: Fn(&mut SettingsApp, String) -> bool + 'static,
{
    cx.subscribe_in(input, window, move |this, state, event, _, cx| {
        if matches!(event, InputEvent::Change) {
            let text = state.read(cx).value().to_string();
            if handler(this, text) {
                this.mark_dirty(cx);
            }
        }
    })
}

fn subscribe_slider<F>(
    cx: &mut Context<SettingsApp>,
    window: &mut Window,
    slider: &Entity<SliderState>,
    setter: F,
) -> Subscription
where
    F: Fn(&mut SettingsApp, f32) -> bool + 'static,
{
    cx.subscribe_in(slider, window, move |this, _state, event, _, cx| {
        if let SliderEvent::Change(value) = event {
            if setter(this, value.start()) {
                this.mark_dirty(cx);
            }
        }
    })
}

fn section_label(label: impl Into<SharedString>, pal: &Palette) -> impl IntoElement {
    div()
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(pal.muted)
        .child(label.into())
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
        .child(div().text_size(px(9.5)).text_color(c(MUTED)).child(label))
}

fn choice_row<T>(
    label: &'static str,
    options: &[(T, &'static str)],
    selected: T,
    pal: &Palette,
    cx: &mut Context<SettingsApp>,
    setter: fn(&mut SettingsApp, T),
) -> impl IntoElement
where
    T: Copy + PartialEq + 'static,
{
    // 两列网格：4 个选项不会把窄栏挤爆，长标签（右肩键 / 按住隐藏）也能完整显示。
    let mut grid = v_flex().gap(px(6.));
    for chunk in options.chunks(2) {
        let mut row = h_flex().gap(px(6.));
        for (value, text) in chunk {
            let value = *value;
            let active = value == selected;
            row = row.child(
                div()
                    .id(format!("choice-{label}-{text}"))
                    .flex_1()
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .bg(if active { pal.accent_soft } else { pal.control })
                    .border_1()
                    .border_color(if active { ca(ACCENT, 0.40) } else { pal.border })
                    .hover(|style| style.bg(pal.card_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        setter(this, value);
                        this.mark_dirty(cx);
                    }))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(if active { pal.text } else { pal.muted })
                            .child(*text),
                    ),
            );
        }
        grid = grid.child(row);
    }
    v_flex()
        .gap(px(7.))
        .child(div().text_size(px(11.5)).text_color(pal.muted).child(label))
        .child(grid)
}

fn nav_glyph(section: Section, active: bool, pal: &Palette) -> impl IntoElement {
    let color = if active { pal.text } else { pal.muted };
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

fn color_value(preset: &Preset, index: usize) -> String {
    match index {
        0 => preset.color.clone(),
        1 => preset.colors.top.clone(),
        2 => preset.colors.bottom.clone(),
        3 => preset.colors.left.clone(),
        4 => preset.colors.right.clone(),
        5 => preset.outline.color.clone(),
        _ => "#FF0000".to_string(),
    }
}

fn normalize_hex(text: &str) -> Option<String> {
    let value = text.trim().trim_start_matches('#');
    if value.len() != 6 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", value.to_ascii_uppercase()))
}

fn pos_string(value: &PosVal) -> String {
    match value {
        PosVal::Center => "0".to_string(),
        PosVal::Px(v) => format!("{v:.0}"),
    }
}

/// GPUI `Keystroke::key` → Windows 虚拟键码。
///
/// 只映射 `Hotkey` 的 Display/parse 能往返的键，和旧 egui 版的 `key_to_vk` 保持一致。
fn key_to_vk(key: &str) -> Option<u32> {
    let key = key.to_ascii_lowercase();
    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        if byte.is_ascii_lowercase() {
            return Some(0x41 + u32::from(byte - b'a'));
        }
        if byte.is_ascii_digit() {
            return Some(0x30 + u32::from(byte - b'0'));
        }
    }
    if let Some(rest) = key.strip_prefix('f') {
        if let Ok(n) = rest.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(0x70 + n - 1);
            }
        }
    }
    Some(match key.as_str() {
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "escape" | "esc" => 0x1B,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" | "ins" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "up" | "arrowup" => 0x26,
        "down" | "arrowdown" => 0x28,
        "left" | "arrowleft" => 0x25,
        "right" | "arrowright" => 0x27,
        "-" | "minus" => 0xBD,
        "=" | "equal" => 0xBB,
        "[" | "bracketleft" => 0xDB,
        "]" | "bracketright" => 0xDD,
        "\\" | "backslash" => 0xDC,
        ";" | "semicolon" => 0xBA,
        "'" | "quote" => 0xDE,
        "," | "comma" => 0xBC,
        "." | "period" => 0xBE,
        "/" | "slash" => 0xBF,
        "`" | "backquote" | "grave" => 0xC0,
        _ => return None,
    })
}

// ===========================================================================
// 自绘
// ===========================================================================

fn paint_preview(
    window: &mut Window,
    bounds: Bounds<gpui_kit::Pixels>,
    preset: &Preset,
    expand: f32,
) {
    let w = f32::from(bounds.size.width);
    let h = f32::from(bounds.size.height);
    let cx = f32::from(bounds.center().x);
    let cy = f32::from(bounds.center().y);
    if w <= 1.0 || h <= 1.0 {
        return;
    }

    let grid = ca(0xFFFFFF, 0.035);
    let step = 32.0_f32;
    let left = f32::from(bounds.origin.x);
    let top = f32::from(bounds.origin.y);
    let mut x = left;
    while x <= left + w {
        paint_line(window, x, top, x, top + h, grid, 1.0);
        x += step;
    }
    let mut y = top;
    while y <= top + h {
        paint_line(window, left, y, left + w, y, grid, 1.0);
        y += step;
    }

    paint_circle(window, cx, cy, (w.min(h) * 0.34).max(40.0), ca(ACCENT, 0.055), true, 0.0);
    paint_circle(window, cx, cy, (w.min(h) * 0.18).max(24.0), ca(ACCENT_2, 0.045), true, 0.0);

    let params = ShapeParams::from_preset(preset, expand);
    let Some(geom) = shapes::build(preset.shape, &params) else {
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
            for (i, height) in [3.5_f32, 6.5, 4.5].iter().enumerate() {
                let x = cx - 5.0 + i as f32 * 5.0;
                paint_line(window, x, cy - height * 0.5, x, cy + height * 0.5, color, stroke);
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

fn paint_gamepad(window: &mut Window, bounds: Bounds<gpui_kit::Pixels>, pal: &Palette) {
    let cx = f32::from(bounds.center().x);
    let cy = f32::from(bounds.center().y);
    paint_rect(window, cx - 60.0, cy - 24.0, 120.0, 48.0, ca(0xFFFFFF, 0.06), 1.2, false);
    paint_circle(window, cx - 32.0, cy, 10.0, pal.accent, false, 1.4);
    paint_circle(window, cx + 32.0, cy, 10.0, pal.accent2, false, 1.4);
    paint_circle(window, cx - 32.0, cy, 4.0, ca(0xFFFFFF, 0.35), true, 0.0);
    paint_circle(window, cx + 32.0, cy, 4.0, ca(0xFFFFFF, 0.35), true, 0.0);
    paint_line(window, cx - 4.0, cy - 10.0, cx + 4.0, cy - 10.0, pal.muted, 2.0);
    paint_line(window, cx, cy - 14.0, cx, cy - 6.0, pal.muted, 2.0);
    paint_line(window, cx - 20.0, cy - 18.0, cx - 12.0, cy - 18.0, pal.dim, 1.2);
    paint_line(window, cx + 12.0, cy - 18.0, cx + 20.0, cy - 18.0, pal.dim, 1.2);
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

// ===========================================================================
// 模板
// ===========================================================================

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

fn template_base(preset: &Preset) -> Preset {
    let mut base = preset.clone();
    base.multicolor = false;
    base.outline.enabled = false;
    base.hollow.gap = 0.0;
    base.hollow.center_dot_size = 0.0;
    base.rotation = 0.0;
    base.opacity = 1.0;
    base.dynamic.fire_expand_px = 0.0;
    base.dynamic.recoil_indicator = false;
    base.image.path.clear();
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(normalize_hex("#ff0000").as_deref(), Some("#FF0000"));
        assert_eq!(normalize_hex("00ff00").as_deref(), Some("#00FF00"));
        assert!(normalize_hex("#f00").is_none());
        assert!(normalize_hex("red").is_none());
    }

    #[test]
    fn key_map_roundtrip() {
        for (key, vk) in [
            ("a", 0x41),
            ("z", 0x5A),
            ("7", 0x37),
            ("f1", 0x70),
            ("f24", 0x87),
            ("space", 0x20),
            ("up", 0x26),
            ("minus", 0xBD),
            ("-", 0xBD),
            ("semicolon", 0xBA),
        ] {
            assert_eq!(key_to_vk(key), Some(vk), "key={key}");
        }
        assert_eq!(key_to_vk("control"), None);
    }

    #[test]
    fn position_strings() {
        assert_eq!(pos_string(&PosVal::Center), "0");
        assert_eq!(pos_string(&PosVal::Px(42.4)), "42");
    }

    #[test]
    fn glass_modes_roundtrip() {
        for mode in [
            GpuiGlass::Acrylic,
            GpuiGlass::Mica,
            GpuiGlass::Transparent,
            GpuiGlass::Opaque,
        ] {
            assert_eq!(GpuiGlass::from_mode(mode.to_mode()), mode);
        }
    }
}
