//! ACAJA 设置窗口（v1.3 「深空玻璃」重设计）。
//!
//! 结构：无边框透明窗口 → 自绘背板 + 自绘标题栏 → 左导航玻璃轨 + 右内容（不对称网格卡片）→ 底部操作条。
//! 视觉：**全部自绘**（深黑灰底 + 低饱和霓虹紫/克莱因蓝色相光斑 + 强毛玻璃叠加 + 镜面边 + 分层阴影），
//!       不依赖系统透明能力——系统透明不可用时界面依然是完整正确的；Win11 上再由
//!       `windowfx::apply_once` 叠加系统圆角 / 亚克力背板（失败静默降级）。
//! 透感（v1.2.2 起）：**双模式底色**——`windowfx::blur_active()` 为真时走 `Palette::translucent(level)`
//!       （基底 alpha 按玻璃强度在 74~150 之间插值，其余层同比缩放，桌面透得进来），
//!       为假时保持不透明底兜底（此时忽略强度）。毛玻璃模式与强度是应用级设置，
//!       在「系统」分区调节、写回 `store.app.glass` 并在启动时恢复。
//!       玻璃"有厚度"靠三件事：跟随鼠标的径向高光（整窗 430px + 每张面板）、
//!       光斑 9px / 面板 2.6px 的**反向视差**、按光照方向（光标侧亮、背面暗）的 1.5px 边缘折射带。
//! 排版：**三族分工**——`FontFamily::Name("serif")` 衬线（标题 / 大字 / 序号）、
//!       `FontFamily::Name("bold")` 无衬线粗体（数值 / 关键值）、`Proportional` 正文；
//!       字体只在 `AcajaApp::new` 里经 `fonts::install` 安装一次（每帧 set_fonts 会闪烁）。
//! 布局：PAD 28 / 侧栏 200 / 内容与侧栏间距 24；内容区走 `row2` 的**不对称两列网格**
//!       （按比例分栏、允许两列不等宽不等高，也允许单卡内部再左右不对称）。
//! 动效（v1.3.1 重调）：全部走 egui 的**缓动**版本（`animate_bool_with_time` 是线性的，
//!       机械感就是它来的）——hover 0.14s / 按下 0.09s / 数值 0.26s / 卡片入场 0.42s
//!       （expo_out，错峰 45ms）/ flash 淡出 0.55s，一律 cubic_out，不做回弹；
//!       只在动画进行中 `request_repaint()`，静止时零重绘。
//! 性能：背板静态层（基底 + 渐变 + 网格 + 噪点）**预烘焙**成一张贴图
//!       （尺寸/主题/透底/DPI 变化时才重算，缩放期间去抖 140ms），每帧只画 1 个矩形；
//!       面板阴影 12 层 → 6 层（铺得更宽补回柔和度）。
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
    Align, Align2, Color32, ComboBox, Context, CursorIcon, FontFamily, FontId, Frame, Id, Layout,
    Margin, Pos2, Rangef, Rect, ResizeDirection, Response, RichText, Rounding, ScrollArea, Sense,
    Shape, Stroke, TextEdit, Ui, UiBuilder, Vec2, ViewportCommand,
};
use log::{info, warn};

use crate::config::{
    AdsButton, AdsMode, GameBinding, GlassMode, Hotkey, PosVal, Preset, PresetStore, RightClickMode,
    Shape as CrossShape,
    MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
};
use crate::i18n::Lang;
use crate::system::monitor::MonitorInfo;
use crate::ui::strings::{ads_mode_name, glass_mode_name, shape_name, t};

const STATUS_TTL: Duration = Duration::from_millis(1900);
/// 实时预览推送的最小间隔：拖动滑杆时约 7 次/秒，足够跟手，也不会打扰主程序
const LIVE_PUSH_MIN: Duration = Duration::from_millis(140);
/// 单次实时推送的超时（毫秒）：主程序忙（例如托盘菜单模态循环）时放弃本次预览，绝不冻结界面
const LIVE_PUSH_TIMEOUT_MS: u32 = 120;

// ---- 布局令牌（v1.3 「深空玻璃」）-----------------------------------------
/// 窗口内边距（大留白：28）
const PAD: f32 = 28.0;
/// 侧栏玻璃轨宽度（比 v1.2 更宽、更"呼吸"）
const NAV_W: f32 = 200.0;
/// 内容区与侧栏的间距
const NAV_GAP: f32 = 24.0;
/// 不对称网格的列间距
const GUTTER: f32 = 20.0;
/// 卡片之间的行间距
const GROUP_GAP: f32 = 24.0;
/// 卡片内边距
const CARD_PAD: f32 = 22.0;
/// 内容宽度低于此值 → 2 列网格退化为单列（避免窄窗下控件溢出）
const STACK_BELOW: f32 = 560.0;
const R_WINDOW: f32 = 20.0;
const R_CARD: f32 = 18.0;
const R_CTRL: f32 = 10.0;

// ---- 动效令牌 -------------------------------------------------------------
// 时长拆成几档（v1.2.1 复盘：一档 0.18s 打天下 → 悬停发木、选中发飘、入场发急）。
// 曲线全部用 egui 自带缓动（`animate_bool_with_time_and_easing`）：
// `animate_bool_with_time` 是 **线性** 的（`context.rs` 里写死了 `easing::linear`），
// 机械感正来自那里。这里统一走 cubic_out，入场走 expo_out；**不做回弹**（工具界面回弹显廉价）。

/// 悬停底色 / 描边 / 抬升
const ANIM_HOVER: f32 = 0.14;
/// 按下反馈（最快的一档，触点即应）
const ANIM_PRESS: f32 = 0.09;
/// 数值 / 勾选 / 导航选中胶囊 / 光条
const ANIM_VALUE: f32 = 0.26;
/// 分区切换 = 卡片入场（切分区就是重置入场时钟让整列卡片重播），
/// 所以只有一档时长：0.42s + 每张 45ms 错峰，曲线 expo_out（前段极快、收尾极软）
const CARD_ENTER_DUR: f32 = 0.42;
/// 卡片错峰入场的间隔
const ENTER_STAGGER: f32 = 0.045;
/// 入场时卡片上移的距离
const ENTER_RISE: f32 = 14.0;
/// hover 抬升距离
const HOVER_LIFT: f32 = 2.0;
/// flash 提示淡入 / 淡出时长（淡出 0.55s，比原来的一条斜线柔和）
const FLASH_IN: f32 = 0.09;
const FLASH_OUT: f32 = 0.55;

// ---- 玻璃动效令牌 ---------------------------------------------------------
/// 鼠标镜面高光的跟随时间常数（秒）：越小越跟手，越大越"重"
const MOUSE_TAU: f32 = 0.07;
/// 跟随收敛阈值（px）：低于它直接吸附到目标，动画结束（不再 request_repaint）
const MOUSE_SETTLE: f32 = 1.2;
/// 背板光斑的视差幅度（px，反向）：光斑动得多 → 与面板拉开前后层次
const BLOB_PARALLAX: f32 = 9.0;
/// 玻璃面板的视差幅度（px）：比光斑小，形成"远动多、近动少"
const PANEL_PARALLAX: f32 = 2.6;
/// 鼠标镜面高光半径（px）：整窗用大半径，面板内按面板尺寸收
const GLOW_RADIUS: f32 = 430.0;
/// 背板预烘焙的去抖时长：拖拽缩放时尺寸每帧都变，等停下再重烘焙
const BAKE_DEBOUNCE: Duration = Duration::from_millis(140);

// ---- 玻璃强度令牌（对应 `config::GlassConfig.level`，用户可在「系统」分区调节）----
// 语义：**值越大越透**。强度只影响基底的透明度，其余层按同一比例缩放；
// 系统模糊不生效时（`blur_active() == false`）整套忽略，走不透明兜底。
/// 强度下限（最实）
const GLASS_MIN: f32 = 0.3;
/// 强度上限（最透）
const GLASS_MAX: f32 = 1.0;
/// 滑杆步进
const GLASS_STEP: f32 = 0.05;
/// 强度 = 1.0 时的基底 alpha（最透）
const GLASS_ALPHA_MAX: f32 = 74.0;
/// 强度 = 0.3 时的基底 alpha（较实）
const GLASS_ALPHA_MIN: f32 = 150.0;
/// 参考：原透明模式硬编码的基底 alpha —— 其余层的缩放系数 = 当前基底 alpha / 该值
const GLASS_ALPHA_REF: f32 = 88.0;

// ---- 字体族助手（族名由 `fonts::install` 保证存在，失败时内部已回退）------

/// 衬线（标题 / 大字 / 序号）
fn f_serif(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(fonts::FAMILY_SERIF.into()))
}

/// 无衬线粗体（数值 / 关键值 / 强调）
fn f_bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(fonts::FAMILY_BOLD.into()))
}

/// 无衬线常规（正文）
fn f_sans(size: f32) -> FontId {
    FontId::proportional(size)
}

/// 常规缓出（hover / 数值 / 分区切换）：起步快、收尾稳
fn ease_out(t: f32) -> f32 {
    egui::emath::easing::cubic_out(t.clamp(0.0, 1.0))
}

/// 进场缓出（卡片入场）：前段极快、后段极软，"落下"而不是"推入"
fn ease_enter(t: f32) -> f32 {
    egui::emath::easing::exponential_out(t.clamp(0.0, 1.0))
}

/// 0→1 的缓动过渡（bool 目标）。egui 0.30 **没有**带缓动的数值版
/// （`animate_value_with_time` 也是线性），所以用 bool 动画 + 缓动读数。
/// 目标为 false 时 egui 会把曲线镜像（`1 - easing(1 - v)`），来回都平滑。
fn anim01(ctx: &Context, id: Id, target: bool, time: f32) -> f32 {
    ctx.animate_bool_with_time_and_easing(id, target, time, egui::emath::easing::cubic_out)
}

/// 带缓出的布尔过渡；动画进行中才会请求重绘（静止时不空转）
fn anim_bool(ctx: &Context, id: Id, target: bool, time: f32) -> f32 {
    anim01(ctx, id, target, time)
}

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

/// 玻璃动效环境：每帧由 `update` 写入。放在 `Palette` 里而不是加参数——
/// `pal` 已经是所有绘制函数的共享上下文，几十个调用点不必再各加一个 `fx` 形参。
#[derive(Clone, Copy)]
struct Fx {
    /// 平滑跟随后的鼠标位置（窗口坐标；鼠标不在窗口里时保持最后位置）
    mouse: Pos2,
    /// 系统模糊是否生效（`windowfx::blur_active()`）：决定底色是"真透"还是"不透明兜底"
    blur: bool,
}

impl Default for Fx {
    fn default() -> Self {
        Self { mouse: Pos2::ZERO, blur: false }
    }
}

/// 深空玻璃调色板。字段全部是 `Color32`（`from_rgba_unmultiplied` 不是 const fn，
/// 所以用运行时构造函数而不是 const 常量），`Copy` 以便随处传值、不借 `self`。
///
/// v1.3 方向：深邃黑灰底（#08080A~#101014，不用纯黑）＋ 低饱和霓虹紫 `#6C5CE7`
/// ＋ 深紫 `#4B3FA8` ＋ 克莱因蓝 `#002FA7`；状态色整体压一档饱和，避免刺眼。
///
/// v1.3.1：底色分**两套**——系统模糊生效时用 [`Palette::translucent`]（真半透明，
/// 桌面透过玻璃可见轮廓与色相），不可用时保持不透明底（界面依旧完整）。
#[derive(Clone, Copy)]
struct Palette {
    /// 本帧玻璃动效环境（鼠标位置 / 系统模糊是否生效）
    fx: Fx,
    dark: bool,
    /// 基底（系统透明不可用时的兜底；`fx.blur` 时是半透明的一层）
    bg_base: Color32,
    bg_top: Color32,
    bg_bottom: Color32,
    /// 光斑：霓虹紫 / 克莱因蓝 / 深紫 / 紫罗兰
    blob_a: Color32,
    blob_b: Color32,
    blob_c: Color32,
    blob_d: Color32,
    grain: Color32,
    /// 玻璃面叠加（很淡，背景色相能从面板里透出来）
    glass: Color32,
    /// 镜面高光（顶亮底暗的外沿）
    glass_hi: Color32,
    glass_lo: Color32,
    border: Color32,
    shadow: Color32,
    text: Color32,
    label: Color32,
    dim: Color32,
    /// 强调色：霓虹紫 → 深紫 → 克莱因蓝
    accent: Color32,
    accent_bright: Color32,
    accent_soft: Color32,
    accent_deep: Color32,
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

/// 玻璃强度 → 真透底基底 alpha：level 1.0（最透）→ [`GLASS_ALPHA_MAX`]，
/// level 0.3（较实）→ [`GLASS_ALPHA_MIN`]，中间线性插值（配置越界也先夹回区间）。
fn glass_alpha(level: f32) -> f32 {
    let t = (level.clamp(GLASS_MIN, GLASS_MAX) - GLASS_MIN) / (GLASS_MAX - GLASS_MIN);
    GLASS_ALPHA_MIN + (GLASS_ALPHA_MAX - GLASS_ALPHA_MIN) * t
}

impl Palette {
    /// 深色（默认）：底 #08080A~#101014/#060608；accent #6C5CE7 + 深紫 #4B3FA8 + 克莱因蓝 #002FA7
    fn dark() -> Self {
        Self {
            fx: Fx::default(),
            dark: true,
            bg_base: rgba(8, 8, 10, 244),
            bg_top: rgba(16, 16, 20, 205),
            bg_bottom: rgba(6, 6, 8, 216),
            blob_a: rgba(108, 92, 231, 104),
            blob_b: rgba(0, 47, 167, 126),
            blob_c: rgba(75, 63, 168, 96),
            blob_d: rgba(108, 92, 231, 54),
            grain: Color32::WHITE,
            glass: rgba(255, 255, 255, 9),
            glass_hi: rgba(255, 255, 255, 41),
            glass_lo: rgba(0, 0, 0, 150),
            border: rgba(255, 255, 255, 26),
            shadow: rgba(0, 0, 0, 165),
            text: Color32::from_rgb(244, 245, 248),
            label: Color32::from_rgb(167, 171, 184),
            dim: Color32::from_rgb(113, 116, 127),
            accent: Color32::from_rgb(108, 92, 231),
            accent_bright: Color32::from_rgb(140, 124, 246),
            accent_soft: rgba(108, 92, 231, 56),
            accent_deep: Color32::from_rgb(75, 63, 168),
            accent2: Color32::from_rgb(0, 47, 167),
            warm: Color32::from_rgb(224, 166, 75),
            control: rgba(255, 255, 255, 22),
            hover: rgba(255, 255, 255, 38),
            input: rgba(0, 0, 0, 130),
            track: rgba(255, 255, 255, 30),
            nav_fg: Color32::from_rgb(142, 147, 163),
            nav_fg_on: Color32::from_rgb(244, 245, 248),
            ok: Color32::from_rgb(78, 203, 113),
            warn: Color32::from_rgb(224, 166, 75),
            danger: Color32::from_rgb(232, 86, 75),
            popup: rgba(12, 12, 16, 250),
            well: rgba(0, 0, 0, 108),
        }
    }

    /// 浅色：雾白 #F7F8FA~#E4E6EE，同色系浅调（克莱因蓝压深作为强调）
    fn light() -> Self {
        Self {
            fx: Fx::default(),
            dark: false,
            bg_base: rgba(246, 247, 250, 250),
            bg_top: rgba(255, 255, 255, 205),
            bg_bottom: rgba(228, 230, 238, 216),
            blob_a: rgba(108, 92, 231, 54),
            blob_b: rgba(0, 47, 167, 40),
            blob_c: rgba(75, 63, 168, 38),
            blob_d: rgba(108, 92, 231, 30),
            grain: Color32::from_rgb(18, 22, 40),
            glass: rgba(255, 255, 255, 205),
            glass_hi: rgba(255, 255, 255, 245),
            glass_lo: rgba(24, 30, 52, 24),
            border: rgba(24, 30, 52, 42),
            shadow: rgba(20, 28, 56, 64),
            text: Color32::from_rgb(23, 26, 34),
            label: Color32::from_rgb(76, 83, 100),
            dim: Color32::from_rgb(122, 128, 144),
            accent: Color32::from_rgb(90, 75, 214),
            accent_bright: Color32::from_rgb(108, 92, 231),
            accent_soft: rgba(90, 75, 214, 40),
            accent_deep: Color32::from_rgb(75, 63, 168),
            accent2: Color32::from_rgb(0, 47, 167),
            warm: Color32::from_rgb(192, 138, 62),
            control: rgba(24, 30, 52, 22),
            hover: rgba(24, 30, 52, 36),
            input: rgba(255, 255, 255, 215),
            track: rgba(24, 30, 52, 34),
            nav_fg: Color32::from_rgb(102, 109, 128),
            nav_fg_on: Color32::from_rgb(16, 19, 26),
            ok: Color32::from_rgb(47, 168, 92),
            warn: Color32::from_rgb(192, 138, 62),
            danger: Color32::from_rgb(212, 75, 64),
            popup: rgba(252, 253, 255, 250),
            well: rgba(24, 30, 52, 18),
        }
    }

    /// **真透底变体**：系统模糊生效（`fx.blur`）时用这一套，桌面透过玻璃仍能看到轮廓与色相。
    ///
    /// `level`（0.3~1.0，**越大越透**）来自用户滑杆：基底 alpha 在 [`GLASS_ALPHA_MAX`]
    /// （level 1.0）与 [`GLASS_ALPHA_MIN`]（level 0.3）之间线性插值，其余层
    /// （渐变 / 光斑 / 玻璃面 / 描边 / 内凹控件 / 阴影）按**同一系数** `k` 同步缩放——
    /// 强度只改"玻璃有多厚"，层与层之间的相对关系保持不变。
    ///
    /// 参考值（深色，k = 1）：基底 88、渐变 54 / 58、玻璃面 14、镜面高光 48、
    /// 光斑 138/150/120/72、描边 34、输入框 148、色井 122、滑杆槽 38、控件底 30、阴影 190；
    /// 浅色用各自的一组参考值、同一个 k。网格 / 噪点强度与系统模糊绑定，
    /// 由 [`bake_backdrop`] 决定。
    ///
    /// 系统模糊**不生效时完全不调用本函数** → 保持不透明底，强度被忽略（安全兜底）。
    fn translucent(mut self, level: f32) -> Self {
        // 统一缩放系数：>1 = 更实（更不透明），<1 = 更透
        let k = glass_alpha(level) / GLASS_ALPHA_REF;
        // 参考 alpha × k 写回；低强度下个别层会顶到 255，这里夹住
        let a8 = |base: f32| (base * k).round().clamp(0.0, 255.0) as u8;
        if self.dark {
            self.bg_base = rgba(8, 8, 10, a8(88.0));
            self.bg_top = rgba(16, 16, 20, a8(54.0));
            self.bg_bottom = rgba(6, 6, 8, a8(58.0));
            self.blob_a = rgba(108, 92, 231, a8(138.0));
            self.blob_b = rgba(0, 47, 167, a8(150.0));
            self.blob_c = rgba(75, 63, 168, a8(120.0));
            self.blob_d = rgba(108, 92, 231, a8(72.0));
            self.glass = rgba(255, 255, 255, a8(14.0));
            self.glass_hi = rgba(255, 255, 255, a8(48.0));
            self.border = rgba(255, 255, 255, a8(34.0));
        } else {
            self.bg_base = rgba(246, 247, 250, a8(150.0));
            self.bg_top = rgba(255, 255, 255, a8(118.0));
            self.bg_bottom = rgba(228, 230, 238, a8(112.0));
            self.blob_a = rgba(108, 92, 231, a8(74.0));
            self.blob_b = rgba(0, 47, 167, a8(58.0));
            self.blob_c = rgba(75, 63, 168, a8(54.0));
            self.blob_d = rgba(108, 92, 231, a8(42.0));
            self.glass = rgba(255, 255, 255, a8(176.0));
            self.glass_hi = rgba(255, 255, 255, a8(226.0));
            self.border = rgba(24, 30, 52, a8(52.0));
        }
        // 内凹控件（输入框 / 滑杆槽 / 色井）在透底上要更明确，否则会"糊"进桌面
        self.input = with_alpha(self.input, a8(if self.dark { 148.0 } else { 204.0 }));
        self.well = with_alpha(self.well, a8(if self.dark { 122.0 } else { 34.0 }));
        self.track = with_alpha(self.track, a8(if self.dark { 38.0 } else { 46.0 }));
        self.control = with_alpha(self.control, a8(30.0));
        self.shadow = with_alpha(self.shadow, a8(if self.dark { 190.0 } else { 88.0 }));
        self
    }

    /// 整块调色板淡入（卡片入场用）：所有 alpha 乘以 `k`
    fn a_mul(self, k: f32) -> Self {
        let f = |c: Color32| with_alpha(c, (c.a() as f32 * k).round().clamp(0.0, 255.0) as u8);
        Self {
            bg_base: f(self.bg_base),
            bg_top: f(self.bg_top),
            bg_bottom: f(self.bg_bottom),
            blob_a: f(self.blob_a),
            blob_b: f(self.blob_b),
            blob_c: f(self.blob_c),
            blob_d: f(self.blob_d),
            grain: f(self.grain),
            glass: f(self.glass),
            glass_hi: f(self.glass_hi),
            glass_lo: f(self.glass_lo),
            border: f(self.border),
            shadow: f(self.shadow),
            text: f(self.text),
            label: f(self.label),
            dim: f(self.dim),
            accent: f(self.accent),
            accent_bright: f(self.accent_bright),
            accent_soft: f(self.accent_soft),
            accent_deep: f(self.accent_deep),
            accent2: f(self.accent2),
            warm: f(self.warm),
            control: f(self.control),
            hover: f(self.hover),
            input: f(self.input),
            track: f(self.track),
            nav_fg: f(self.nav_fg),
            nav_fg_on: f(self.nav_fg_on),
            ok: f(self.ok),
            warn: f(self.warn),
            danger: f(self.danger),
            popup: f(self.popup),
            well: f(self.well),
            ..self
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

// ---- 鼠标 / 视差 ----------------------------------------------------------

/// 鼠标在 `rect` 内的归一化位置（各轴 -1..1；鼠标不在窗口里时用最后位置）
fn mouse_rel(pal: &Palette, rect: Rect) -> Vec2 {
    let c = rect.center();
    Vec2::new(
        ((pal.fx.mouse.x - c.x) / (rect.width() * 0.5).max(1.0)).clamp(-1.0, 1.0),
        ((pal.fx.mouse.y - c.y) / (rect.height() * 0.5).max(1.0)).clamp(-1.0, 1.0),
    )
}

/// 视差位移：与鼠标**反向**、幅度 `amp`（px）。
///
/// 背板光斑取 9px、面板取 2.6px —— 远的动得多、近的动得少，
/// 鼠标划过时两层相对错动，"厚度"就是这么来的。
fn parallax(pal: &Palette, rect: Rect, amp: f32) -> Vec2 {
    let r = mouse_rel(pal, rect);
    Vec2::new(-r.x * amp, -r.y * amp)
}

/// 直通 alpha 的 over 合成（`src` 叠在 `dst` 之上）。
///
/// `Color32` 存的是预乘 alpha，所以直接按预乘公式相加即可；
/// 通道再夹到结果 alpha 以内，避免舍入让预乘不变量被破坏（epaint 会 debug_assert）。
fn over(dst: Color32, src: Color32) -> Color32 {
    let sa = src.a() as u32;
    let inv = 255 - sa;
    let a = sa + dst.a() as u32 * inv / 255;
    let ch = |s: u8, d: u8| ((s as u32 + d as u32 * inv / 255).min(a)) as u8;
    Color32::from_rgba_premultiplied(ch(src.r(), dst.r()), ch(src.g(), dst.g()), ch(src.b(), dst.b()), a as u8)
}

// ===========================================================================
// 玻璃绘制原语（全部产出 Shape，便于放到「内容之下」或一次性提交）
// ===========================================================================

/// 柔和外阴影：多层同心圆角描边（egui 没有高斯模糊，用同心层模拟）。
///
/// v1.3：深色主题下铺到 34px、12 层，再加一层 3px 的贴身暗部——
/// 卡片被"托"在深空底之上，而不是贴上去。
///
/// v1.3.1（性能）：层数 12 → **6**、铺开范围 34 → **42px**，指数从 2.0 放宽到 1.7。
/// 单层更宽更柔，叠出来的观感几乎不变，但每帧的描边数量减半——
/// 阴影是每张卡、每个按钮都在画的，这一项直接决定滚动时的帧时间。
fn push_shadow(out: &mut Vec<Shape>, rect: Rect, radius: f32, pal: &Palette, k: f32) {
    let spread = if pal.dark { 42.0 } else { 26.0 };
    let steps = if pal.dark { 6 } else { 5 };
    let step = spread / steps as f32;
    let y = if pal.dark { 10.0 } else { 5.0 };
    for i in 0..steps {
        let t = i as f32 / steps as f32;
        let grow = 1.0 + i as f32 * step;
        let a = (1.0 - t).powf(1.7) * 0.10 * k;
        if a <= 0.004 {
            continue;
        }
        out.push(Shape::rect_stroke(
            rect.expand(grow).translate(Vec2::new(0.0, y * (0.35 + 0.65 * t))),
            Rounding::same(radius + grow),
            Stroke::new(step + 0.6, fade(pal.shadow, a)),
        ));
    }
    // 贴身暗部：给玻璃底座一个明确的落点
    out.push(Shape::rect_stroke(
        rect.expand(0.5).translate(Vec2::new(0.0, 3.0)),
        Rounding::same(radius + 0.5),
        Stroke::new(2.0_f32, fade(pal.shadow, 0.22 * k)),
    ));
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

/// 玻璃面板：外阴影 → 白色玻璃底 → **紫/克莱因蓝色相流动** → 顶部光泽 → 镜面边（顶亮底暗）
/// → 跟随鼠标的径向高光 + 随光照方向的边缘折射带。
///
/// 关键取舍：玻璃底保持很低的 alpha（≈0.05），背景的光斑色相能从面板里透出来；
/// 层次靠"叠加"而不是"加厚"——多叠几层极淡的色，而不是把面板画得更不透明。
///
/// v1.3.1（"玻璃要有厚度"）：面板整体做 2.6px 的反向视差（内容不动 → 前后分层），
/// 再叠一层**被面板周长裁住**的鼠标径向高光；镜面边之外按光照方向补 1.5px 亮 / 暗带。
fn push_panel(out: &mut Vec<Shape>, rect: Rect, radius: f32, pal: &Palette, hot: f32) {
    // 视差：玻璃微移，内容不动 → 鼠标划过时面板与背板错开，产生厚度
    let rect = rect.translate(parallax(pal, rect, PANEL_PARALLAX));
    push_shadow(out, rect, radius, pal, 0.9 + 0.5 * hot);
    if pal.dark {
        // 白色玻璃底：上微亮、下沉
        push_vgrad(out, rect, radius, rgba(255, 255, 255, 15), pal.glass, 6);
        // 色相流动：顶缘霓虹紫 → 下半克莱因蓝（极淡，只负责"有色"）
        push_vgrad(
            out,
            rect,
            radius,
            fade(pal.accent, 0.075 + 0.035 * hot),
            fade(pal.accent2, 0.065),
            8,
        );
        if hot > 0.01 {
            // hover：面板内部泛起的紫色呼吸 + 内侧一圈柔光
            out.push(Shape::rect_filled(rect, Rounding::same(radius), fade(pal.accent_soft, 0.22 * hot)));
            out.push(Shape::rect_stroke(
                rect.shrink(1.0),
                Rounding::same(radius - 1.0),
                Stroke::new(1.5_f32, fade(pal.accent, 0.10 * hot)),
            ));
        }
    } else {
        push_vgrad(out, rect, radius, rgba(255, 255, 255, 235), pal.glass, 6);
        push_vgrad(out, rect, radius, fade(pal.accent, 0.035), fade(pal.accent2, 0.030), 8);
    }
    // 顶部光泽带（固定高度，底边是直线、内部不溢出）
    let gloss = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.top() + 56.0));
    out.push(Shape::rect_filled(
        gloss,
        Rounding { nw: radius, ne: radius, sw: 0.0, se: 0.0 },
        fade(pal.glass_hi, if pal.dark { 0.34 + 0.18 * hot } else { 0.55 + 0.20 * hot }),
    ));
    // 鼠标镜面高光：中心最亮、沿半径平方衰减；外环取圆角周长 → **照不到面板外面**
    push_radial_glow(
        out,
        rect,
        radius,
        GLOW_RADIUS,
        pal.fx.mouse,
        pal.glass_hi,
        if pal.dark { 0.075 } else { 0.06 },
    );
    // 镜面边：1px 外沿（hover 泛紫）
    out.push(Shape::rect_stroke(
        rect,
        Rounding::same(radius),
        Stroke::new(1.0_f32, mix(pal.border, pal.accent_bright, 0.40 * hot)),
    ));
    // 边缘折射：1px 镜面边之外再补 1.5px 的亮 / 暗带。
    // 光照方向跟随光标（朝光的一侧亮、背光一侧暗），鼠标移动时光带在四边之间连续过渡。
    let inner = Rangef::new(rect.left() + radius * 0.75, rect.right() - radius * 0.75);
    let vinner = Rangef::new(rect.top() + radius * 0.7, rect.bottom() - radius * 0.7);
    let rel = mouse_rel(pal, rect);
    let (wl, wr) = ((0.5 - 0.5 * rel.x).max(0.0), (0.5 + 0.5 * rel.x).max(0.0));
    let (wt, wb) = ((0.5 - 0.5 * rel.y).max(0.0), (0.5 + 0.5 * rel.y).max(0.0));
    let (bw, k) = (1.5_f32, 0.9 + 0.1 * hot);
    let hi = |w: f32| fade(pal.glass_hi, (0.22 + 0.68 * w) * k);
    let lo = |w: f32| fade(pal.glass_lo, 0.25 + 0.65 * w);
    out.push(Shape::hline(inner, rect.top() + 1.5, Stroke::new(bw, hi(wt))));
    out.push(Shape::hline(inner, rect.bottom() - 1.5, Stroke::new(bw, lo(wb))));
    out.push(Shape::vline(rect.left() + 1.5, vinner, Stroke::new(bw, hi(wl * 0.8))));
    out.push(Shape::vline(rect.right() - 1.5, vinner, Stroke::new(bw, lo(wr * 0.9))));
}

/// 内凹玻璃（输入框 / 下滑槽 / 缩略图井）：顶内阴影 + 底玻璃厚度
fn push_well(out: &mut Vec<Shape>, rect: Rect, radius: f32, pal: &Palette) {
    out.push(Shape::rect_filled(rect, Rounding::same(radius), pal.input));
    // 下半段再压一层暗色 → 内凹的纵深
    let lower = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + rect.height() * 0.5), rect.max);
    out.push(Shape::rect_filled(
        lower,
        Rounding { nw: 0.0, ne: 0.0, sw: radius, se: radius },
        pal.well,
    ));
    let inner = Rangef::new(rect.left() + radius * 0.6, rect.right() - radius * 0.6);
    // 顶：内阴影（转折）
    out.push(Shape::hline(inner, rect.top() + 1.0, Stroke::new(2.0_f32, fade(pal.glass_lo, 0.55))));
    // 底：玻璃厚度的一道细高光
    out.push(Shape::hline(inner, rect.bottom() - 1.0, Stroke::new(1.0_f32, fade(pal.glass_hi, 0.12))));
    out.push(Shape::rect_stroke(rect, Rounding::same(radius), Stroke::new(1.0_f32, fade(pal.border, 0.9))));
}

/// 背板：预烘焙贴图（基底 + 渐变 + 网格 + 噪点）→ 跟随鼠标的色相光斑（视差）
/// → 鼠标镜面高光 → 玻璃窗镜面轮廓。
///
/// 静态层以前每帧要重画 ~230 个图元（30 段渐变 + 网格线 + 200 个噪点 + 14 环光斑），
/// v1.3.1 起一次性栅格化成贴图（见 [`bake_backdrop`]）——
/// 每帧只剩 1 张图 + 5 个光斑网格（原来 58 个圆）+ 1 个鼠标高光网格 + 1px 描边。
fn paint_backdrop(p: &egui::Painter, rect: Rect, pal: &Palette, tex: Option<egui::TextureId>) {
    match tex {
        Some(id) => {
            p.image(id, rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            p.rect_filled(rect, Rounding::same(R_WINDOW), pal.bg_base);
        }
    }
    // 大面积柔和色相光斑：霓虹紫 / 克莱因蓝 / 深紫 —— 卡片就浮在它们上面，
    // 玻璃面板很透，所以这些色相会从每一张卡里透出来（"毛玻璃透感"的来源）。
    // 光斑随鼠标做 9px 的反向视差（面板 2.6px）——两层错动，鼠标划过时就有前后层次。
    let (w, h) = (rect.width(), rect.height());
    let ofs = parallax(pal, rect, BLOB_PARALLAX);
    let at = |x: f32, y: f32| Pos2::new(rect.left() + x, rect.top() + y) + ofs;
    let mut shapes = Vec::new();
    // 峰值 = 原「同心圆叠加」的等效累积 alpha（14 环 → 0.78α、12 环 → 0.72α、10 环 → 0.65α、8 环 → 0.58α）
    push_blob(&mut shapes, rect, at(w * 0.10, h * 0.01), w * 0.54, pal.blob_a, 0.78);
    push_blob(&mut shapes, rect, at(w * 1.00, h * 0.24), w * 0.48, pal.blob_b, 0.78);
    push_blob(&mut shapes, rect, at(w * 0.70, h * 1.05), w * 0.46, pal.blob_c, 0.72);
    push_blob(&mut shapes, rect, at(w * 0.26, h * 0.97), w * 0.32, pal.blob_d, 0.65);
    push_blob(&mut shapes, rect, at(w * 0.88, h * 0.74), w * 0.24, pal.blob_a, 0.58);
    // 鼠标镜面高光（整窗）：大半径 430px、极低 alpha，平滑跟随光标
    push_radial_glow(
        &mut shapes,
        rect,
        R_WINDOW,
        GLOW_RADIUS,
        pal.fx.mouse,
        pal.glass_hi,
        if pal.dark { 0.05 } else { 0.045 },
    );
    p.add(Shape::Vec(shapes));
    // 玻璃窗镜面轮廓 + 顶部一道更亮的高光
    p.rect_stroke(rect.shrink(0.5), Rounding::same(R_WINDOW), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.5)));
    p.line_segment(
        [Pos2::new(rect.left() + R_WINDOW, rect.top() + 0.5), Pos2::new(rect.right() - R_WINDOW, rect.top() + 0.5)],
        Stroke::new(1.0_f32, fade(pal.glass_hi, 0.75)),
    );
}

/// 圆角矩形周长的采样点（顺时针，每个圆角 `seg` 段）。
///
/// 用作径向高光的外环：高光被**面板形状**裁住，不会溢出去照亮面板外面的东西
/// （egui 没有圆角裁剪，普通圆形光晕在圆角处会"漏"出去）。
fn rounded_perimeter(rect: Rect, radius: f32, seg: usize, out: &mut Vec<Pos2>) {
    use std::f32::consts::{FRAC_PI_2, PI};
    let r = radius.clamp(0.0, rect.width().min(rect.height()) * 0.5);
    if r <= 0.01 {
        out.extend([rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]);
        return;
    }
    let seg = seg.max(2);
    let corners = [
        (Pos2::new(rect.left() + r, rect.top() + r), PI),
        (Pos2::new(rect.right() - r, rect.top() + r), PI + FRAC_PI_2),
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0),
        (Pos2::new(rect.left() + r, rect.bottom() - r), FRAC_PI_2),
    ];
    for (c, a0) in corners {
        for i in 0..=seg {
            let a = a0 + FRAC_PI_2 * (i as f32 / seg as f32);
            out.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
}

/// 跟随鼠标的柔和径向高光：顶点扇形，中心最亮，外环按「到光心的距离」平方衰减。
///
/// 外环取**圆角矩形周长**（`corner` = 该矩形的圆角半径）→ 高光被形状裁住，
/// 既不会溢到面板外面照亮别的东西，也不会在窗口的透明圆角处漏出一块颜色
/// （egui 没有圆角裁剪，普通圆形光晕在圆角上一定会漏）。
///
/// 半径取 `min(radius, 面板长边 × 0.9)`——缩略图这种小面板里也要有衰减，
/// 否则整块一起亮就变成了"泛白"而不是高光。
fn push_radial_glow(
    out: &mut Vec<Shape>,
    rect: Rect,
    corner: f32,
    radius: f32,
    center: Pos2,
    color: Color32,
    alpha: f32,
) {
    if alpha <= 0.004 || rect.width() < 6.0 || rect.height() < 6.0 {
        return;
    }
    let center = Pos2::new(center.x.clamp(rect.left(), rect.right()), center.y.clamp(rect.top(), rect.bottom()));
    let r = radius.min(rect.width().max(rect.height()) * 0.9).max(24.0);
    let mut per = Vec::with_capacity(32);
    rounded_perimeter(rect, corner.min(rect.width().min(rect.height()) * 0.5), 5, &mut per);
    let mut mesh = egui::epaint::Mesh::default();
    let a8 = |f: f32| (alpha * f * 255.0).round().clamp(0.0, 255.0) as u8;
    mesh.colored_vertex(center, with_alpha(color, a8(1.0)));
    for p in &per {
        let d = (*p - center).length();
        let f = (1.0 - d / r).clamp(0.0, 1.0);
        mesh.colored_vertex(*p, with_alpha(color, a8(f * f)));
    }
    let n = per.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    out.push(Shape::mesh(mesh));
}

/// 色相光斑：同一个扇形网格，但落在一个**固定的峰值 alpha** 上，向边缘平方衰减。
///
/// v1.3.1 之前是「14 层同心圆叠加」，峰值 alpha ≈ `1 - Π(1-aᵢ) ≈ 0.62`，
/// 换成一个网格后：形状数从 14 降到 1，且能像窗口圆角那样裁剪（同心圆做不到），
/// 观感上更接近真正的高斯光斑（顶点插值比离散同心圆更平滑）。
fn push_blob(out: &mut Vec<Shape>, win: Rect, center: Pos2, radius: f32, color: Color32, peak: f32) {
    push_radial_glow(out, win, R_WINDOW, radius, center, color, peak * color.a() as f32 / 255.0);
}

/// 按系数缩放像素（预乘 alpha 的四个通道一起缩，保持预乘不变量）
fn scale_px(c: Color32, f: f32) -> Color32 {
    let f = f.clamp(0.0, 1.0);
    let s = |v: u8| (v as f32 * f).round() as u8;
    Color32::from_rgba_premultiplied(s(c.r()), s(c.g()), s(c.b()), s(c.a()))
}

/// 背板预烘焙：把「基底 + 垂直渐变 + 网格 + 噪点」这些**静态**层一次性栅格化成 RGBA 像素。
///
/// - 触发条件（任一变化）：窗口像素尺寸 / 主题 / **系统模糊是否生效** / DPI 缩放；
/// - 尺寸连续变化（拖拽缩放）时按 [`BAKE_DEBOUNCE`] 去抖，期间的旧贴图被拉伸，
///   一帧的事，看不出来；
/// - 贴图尺寸 = 逻辑尺寸 × `pixels_per_point`（1:1 texel，颗粒与网格线不糊）；
/// - 圆角在**烘焙时**就抹成透明 → 画一张矩形图就是窗口形状，四角不会露方角。
fn bake_backdrop(size: [usize; 2], pal: &Palette, ppp: f32) -> egui::ColorImage {
    let [w, h] = size;
    let mut img = egui::ColorImage::new(size, Color32::TRANSPARENT);
    if w == 0 || h == 0 {
        return img;
    }
    // 确定性噪点瓦片（256×256）：同一种子 → 静态颗粒，绝不逐帧随机（否则会闪）
    let mut noise = [0u8; 256 * 256];
    let mut seed = 0x9E37_79B9u32;
    for v in noise.iter_mut() {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *v = (seed >> 24) as u8;
    }
    let ppp = ppp.max(0.5);
    let grid_step = (36.0 * ppp).max(6.0);
    let line_w = ppp.max(1.0);
    // 透底模式下网格 / 噪点加强（叠在真实桌面上时弱了会看不见层次）
    let (grid_a, grain_a) = match (pal.dark, pal.fx.blur) {
        (true, true) => (0.045_f32, 0.030_f32),
        (true, false) => (0.026_f32, 0.018_f32),
        (false, true) => (0.060_f32, 0.038_f32),
        (false, false) => (0.040_f32, 0.025_f32),
    };
    let grid = with_alpha(pal.grain, (grid_a * 255.0).round() as u8);
    let grain = grain_a * 255.0;
    // 哪些列 / 行落在网格线上（预计算，免得逐像素取模）
    let gx: Vec<bool> = (0..w).map(|x| (x as f32 % grid_step) < line_w).collect();
    let gy: Vec<bool> = (0..h).map(|y| (y as f32 % grid_step) < line_w).collect();
    let r = R_WINDOW * ppp;
    let (fw, fh) = (w as f32, h as f32);
    for y in 0..h {
        let gy_f = (y as f32 + 0.5) / fh;
        // 基底 + 垂直渐变：整行只算一次（这是 2M 像素里唯一"便宜"的部分）
        let row0 = over(pal.bg_base, mix(pal.bg_top, pal.bg_bottom, gy_f));
        let row = if gy[y] { over(row0, grid) } else { row0 };
        let py = y as f32 + 0.5;
        let dy = if py < r { py - r } else if py > fh - r { py - (fh - r) } else { 0.0 };
        for x in 0..w {
            let mut c = if gx[x] { over(row, grid) } else { row };
            let n = noise[((y & 255) << 8) | (x & 255)];
            if n > 8 {
                c = over(c, with_alpha(pal.grain, (grain * n as f32 / 255.0) as u8));
            }
            // 圆角遮罩：四角之外完全透明（只有角落那几平方百像素会算这段）
            if dy != 0.0 {
                let px = x as f32 + 0.5;
                let dx = if px < r { px - r } else if px > fw - r { px - (fw - r) } else { 0.0 };
                if dx != 0.0 {
                    let d = (dx * dx + dy * dy).sqrt();
                    if d >= r {
                        img.pixels[y * w + x] = Color32::TRANSPARENT;
                        continue;
                    }
                    c = scale_px(c, r - d);
                }
            }
            img.pixels[y * w + x] = c;
        }
    }
    img
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
    // 滚动条也走玻璃语言：细、圆、无凹槽（ScrollStyle 挂在 spacing 上，不在 visuals）
    {
        let mut sc = egui::style::ScrollStyle::floating();
        sc.bar_width = 9.0;
        sc.floating_width = 4.0;
        sc.handle_min_length = 36.0;
        sc.bar_inner_margin = 4.0;
        ui.style_mut().spacing.scroll = sc;
    }
    // 滚动平滑：滚轮 / 拖拽走 egui 的 `smooth_scroll_delta`（本来就是插值的），
    // 这里再把**程序化滚动**（点到控件自动滚到可见）从默认的 1000px/s 调慢一点，
    // 时长区间也放宽到 0.14~0.34s —— 一格格跳的感觉来自程序化滚动这条路径。
    ui.style_mut().scroll_animation = egui::style::ScrollAnimation::new(760.0, Rangef::new(0.14, 0.34));
    let v = ui.visuals_mut();
    v.dark_mode = pal.dark;
    v.override_text_color = Some(pal.text);
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = pal.popup;
    v.window_stroke = Stroke::new(1.0_f32, mix(pal.border, pal.accent_bright, 0.18));
    v.window_rounding = Rounding::same(14.0);
    v.menu_rounding = Rounding::same(14.0);
    v.extreme_bg_color = pal.input;
    v.faint_bg_color = pal.control;
    v.selection.bg_fill = pal.accent_soft;
    v.selection.stroke = Stroke::new(1.0_f32, pal.accent_bright);
    v.window_shadow = egui::epaint::Shadow {
        offset: Vec2::new(0.0, 14.0),
        blur: 30.0,
        spread: 0.0,
        color: fade(pal.shadow, 0.55),
    };
    v.popup_shadow = v.window_shadow;
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
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, mix(pal.border, pal.accent_bright, 0.7));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.hovered.rounding = Rounding::same(R_CTRL);
    v.widgets.active.bg_fill = pal.accent_soft;
    v.widgets.active.weak_bg_fill = pal.accent_soft;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, pal.accent);
    v.widgets.active.fg_stroke = Stroke::new(1.0_f32, pal.text);
    v.widgets.active.rounding = Rounding::same(R_CTRL);
    v.widgets.open.bg_fill = pal.control;
    v.widgets.open.weak_bg_fill = pal.control;
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, fade(pal.accent, 0.5));
    v.widgets.open.rounding = Rounding::same(R_CTRL);
}

/// 玻璃按钮。`min_w = 0` → 按文字自动宽度。
///
/// 动效：底色 / 描边 / 位移（抬升 2px）共用同一条 `hot` 曲线（0.18s ease-out），
/// 三项同进同出，不会出现"某一项先跳、另一项慢半拍"的生硬感。
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
    let font = match kind {
        Btn::Primary => f_bold(font_size),
        _ => f_sans(font_size),
    };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), pal.text);
    let w = (galley.size().x + 26.0).max(min_w);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let id = Id::new(("btn", seed, label));
    let hot = anim_bool(ui.ctx(), id, resp.hovered(), ANIM_HOVER);
    // 按下反馈：0.09s 的 quadratic 快曲线（触点即应），位移与底色同一条曲线
    let press = ui
        .ctx()
        .animate_bool_with_time_and_easing(id.with("press"), resp.is_pointer_button_down_on(), ANIM_PRESS, egui::emath::easing::quadratic_out);
    let lift = HOVER_LIFT * hot - 1.2 * press;
    let rect = rect.translate(Vec2::new(0.0, -lift));
    let radius = Rounding::same(R_CTRL);
    let fg = if pal.dark { pal.text } else { Color32::WHITE };
    let mut shapes = Vec::new();
    match kind {
        Btn::Primary => {
            push_shadow(&mut shapes, rect, R_CTRL, pal, 0.65 + 0.55 * hot);
            push_vgrad(
                &mut shapes,
                rect,
                R_CTRL,
                mix(mix(pal.accent_bright, pal.accent, 0.30), Color32::WHITE, 0.06 + 0.12 * hot),
                mix(pal.accent_deep, pal.accent2, 0.40),
                5,
            );
            shapes.push(Shape::hline(
                Rangef::new(rect.left() + R_CTRL, rect.right() - R_CTRL),
                rect.top() + 1.2,
                Stroke::new(1.0_f32, rgba(255, 255, 255, 104)),
            ));
            shapes.push(Shape::rect_stroke(rect, radius, Stroke::new(1.0_f32, fade(Color32::WHITE, 0.16 + 0.22 * hot))));
            ui.painter().add(Shape::Vec(shapes));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, fg);
        }
        Btn::Danger => {
            push_shadow(&mut shapes, rect, R_CTRL, pal, 0.5 + 0.5 * hot);
            push_vgrad(
                &mut shapes,
                rect,
                R_CTRL,
                mix(pal.danger, Color32::WHITE, 0.12 + 0.10 * hot),
                mix(pal.danger, Color32::BLACK, 0.12),
                4,
            );
            shapes.push(Shape::rect_stroke(rect, radius, Stroke::new(1.0_f32, fade(pal.danger, 0.95))));
            ui.painter().add(Shape::Vec(shapes));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, fg);
        }
        Btn::Glass => {
            push_shadow(&mut shapes, rect, R_CTRL, pal, 0.35 + 0.45 * hot);
            push_vgrad(
                &mut shapes,
                rect,
                R_CTRL,
                mix(pal.control, pal.glass_hi, 0.06 + 0.10 * hot),
                mix(pal.control, pal.glass_lo, 0.22),
                4,
            );
            shapes.push(Shape::rect_stroke(
                rect,
                radius,
                Stroke::new(1.0_f32, mix(pal.border, pal.accent_bright, hot * 0.75)),
            ));
            shapes.push(Shape::hline(
                Rangef::new(rect.left() + R_CTRL, rect.right() - R_CTRL),
                rect.top() + 1.0,
                Stroke::new(1.0_f32, fade(pal.glass_hi, 0.30 + 0.25 * hot)),
            ));
            ui.painter().add(Shape::Vec(shapes));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, mix(pal.label, pal.text, hot));
        }
        Btn::Ghost => {
            // 幽灵按钮：hover 才浮出，底色 / 描边 / 文字同一条曲线
            if hot > 0.01 {
                shapes.push(Shape::rect_filled(rect, radius, fade(pal.control, 0.2 + 0.8 * hot)));
                shapes.push(Shape::rect_stroke(
                    rect,
                    radius,
                    Stroke::new(1.0_f32, fade(mix(pal.border, pal.accent_bright, 0.55), hot)),
                ));
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

/// 勾选框：accent 填充 + 几何对勾（勾选 / hover 两条独立但同步的缓动）
fn checkbox(ui: &mut Ui, pal: &Palette, seed: &str, on: &mut bool, label: &str) -> bool {
    let font = f_sans(12.5);
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), font.clone(), pal.label).size().x;
    let w = (26.0 + text_w).min(ui.available_width().max(26.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
    let t = anim01(ui.ctx(), Id::new(("chk", seed)), *on, ANIM_VALUE);
    let hot = anim_bool(ui.ctx(), Id::new(("chkh", seed)), resp.hovered(), ANIM_HOVER);
    let box_rect = Rect::from_center_size(Pos2::new(rect.left() + 9.0, rect.center().y), Vec2::splat(18.0));
    let p = ui.painter();
    p.rect_filled(box_rect, Rounding::same(6.0), pal.input);
    if hot > 0.01 {
        p.rect_filled(box_rect.expand(2.5), Rounding::same(8.0), fade(pal.accent, 0.14 * hot));
    }
    if t > 0.01 {
        // 填充色随 t 连续过渡（input → accent），不是开关式跳变
        p.rect_filled(box_rect, Rounding::same(6.0), mix(pal.input, pal.accent, t));
        let c = box_rect.center();
        p.add(Shape::line(
            vec![Pos2::new(c.x - 3.9, c.y + 0.2), Pos2::new(c.x - 1.1, c.y + 3.1), Pos2::new(c.x + 4.2, c.y - 3.2)],
            Stroke::new(2.0_f32, fade(Color32::WHITE, t)),
        ));
    }
    p.rect_stroke(
        box_rect,
        Rounding::same(6.0),
        Stroke::new(1.0_f32, mix(mix(pal.border, pal.accent, t), pal.accent_bright, hot * 0.75)),
    );
    p.text(
        Pos2::new(box_rect.right() + 9.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        mix(mix(pal.label, pal.text, t), pal.text, hot * 0.8),
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

/// 细轨道 + 高光把手滑杆（f32）；数值用 **bold** 族，与标签形成粗细对比
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
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(60.0), 22.0), Sense::click_and_drag());
    let val_w = 46.0_f32.min(rect.width() * 0.45);
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
    let hot = anim_bool(ui.ctx(), Id::new(("sl", seed)), resp.hovered() || resp.dragged(), ANIM_HOVER);
    let p = ui.painter();
    p.rect_filled(track.expand2(Vec2::new(0.0, 0.5)), Rounding::same(2.5), pal.track);
    let fill = Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * t, track.max.y));
    if fill.width() > 0.5 {
        p.rect_filled(
            fill.expand2(Vec2::new(0.0, 0.5)),
            Rounding::same(2.5),
            mix(pal.accent, pal.accent_bright, 0.35 + 0.35 * hot),
        );
    }
    let c = Pos2::new(track.left() + track.width() * t, rect.center().y);
    let r = 5.6 + 1.5 * hot;
    p.circle_filled(c, r + 5.0, fade(pal.accent, 0.12 + 0.26 * hot));
    p.circle_filled(c, r, Color32::WHITE);
    p.circle_stroke(c, r, Stroke::new(1.0_f32, fade(pal.accent, 0.8 + 0.2 * hot)));
    p.text(
        Pos2::new(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        format!("{:.*}{}", decimals, *v, suffix),
        f_bold(11.5),
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

/// 字段标签列的默认宽度：中英双语都塞得下最长的一个（"Threshold (0-255)" ≈ 112px）
const LABEL_W: f32 = 122.0;

/// 标签列宽：窄栏（不对称网格的窄列）自动收窄，避免控件被挤出卡片
fn label_w(ui: &Ui) -> f32 {
    let a = ui.available_width();
    if a < 250.0 {
        82.0
    } else if a < 330.0 {
        104.0
    } else {
        LABEL_W
    }
}

/// 固定宽度的字段标签（对齐用）；超出列宽的部分裁掉，绝不压到右侧控件上
fn field_label(ui: &mut Ui, pal: &Palette, text: &str) {
    let w = label_w(ui);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::hover());
    let clip = rect.intersect(ui.clip_rect());
    let p = ui.painter().with_clip_rect(clip);
    p.text(rect.left_center(), Align2::LEFT_CENTER, text, f_sans(12.5), pal.label);
}

/// 说明文字（弱色 10.5，随栏宽自动折行）
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
///
/// 动效（都走 ease-out，绝无生硬跳变）：
/// - **入场**：`enter` ∈ [0,1] 由调用方按区块计时（首次出现 / 切换分区时只播一次），
///   0.30s 淡入 + 上移 14px；整卡（玻璃 + 内容）一起位移，靠"先负间距、画完补回"实现，
///   布局高度因此不变（不会把后面的卡片推来推去）。
/// - **hover**：整卡抬升 2px + 面板内部泛紫 + 阴影加深，同一条 0.18s 曲线。
///   抬升量取上一帧的 hover 值——内容与玻璃位移完全同步，滚动时也不会抖。
fn card(
    ui: &mut Ui,
    pal: &Palette,
    seed: &str,
    num: &str,
    enter: f32,
    title: Option<&str>,
    body: impl FnOnce(&mut Ui, &Palette),
) {
    let e = ease_enter(enter);
    let id = Id::new(("card", seed));
    let hid = Id::new(("cardh", seed));
    let prev: Option<Rect> = ui.ctx().data(|d| d.get_temp(id));
    let hovered = prev.is_some_and(|r| ui.rect_contains_pointer(r));
    let hot_prev: f32 = ui.ctx().data(|d| d.get_temp(hid)).unwrap_or(0.0);
    // 整卡位移 = 入场上移 + hover 抬升（入场期间 hover 抬升按 e 淡入，两段位移不叠加打架）
    let up = ENTER_RISE * (1.0 - e) + HOVER_LIFT * hot_prev * e;
    if up > 0.05 {
        ui.add_space(-up);
    }
    // 整卡淡入：把调色板整体乘一次 alpha，面板与文字一起渐显
    let cp = pal.a_mul(0.10 + 0.90 * e);
    let width = ui.available_width();
    let slot = ui.painter().add(Shape::Noop);
    let resp = Frame::none()
        .inner_margin(Margin::same(CARD_PAD))
        .show(ui, |ui| {
            ui.set_width((width - CARD_PAD * 2.0).max(120.0));
            ui.spacing_mut().item_spacing = Vec2::new(10.0, 12.0);
            if let Some(title) = title {
                card_title(ui, &cp, num, title);
                ui.add_space(6.0);
            }
            body(ui, &cp);
            ui.add_space(4.0);
        });
    if up > 0.05 {
        ui.add_space(up);
    }
    let rect = resp.response.rect;
    let hot = anim_bool(ui.ctx(), hid, hovered, ANIM_HOVER);
    let mut shapes = Vec::new();
    // 面板也走淡入调色板：玻璃与内容一起渐显（否则玻璃会先跳出来）
    push_panel(&mut shapes, rect, R_CARD, &cp, hot);
    ui.painter().set(slot, Shape::Vec(shapes));
    ui.ctx().data_mut(|d| {
        d.insert_temp(id, rect);
        d.insert_temp(hid, hot);
    });
}

/// 卡片标题：衬线序号（"i/n"，n = 本区卡片数；单卡分区传空串）+ 衬线标题 + 固定宽度细线
fn card_title(ui: &mut Ui, pal: &Palette, num: &str, title: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter();
    let mut x = rect.left();
    if !num.is_empty() {
        let ng = p.layout_no_wrap(num.to_owned(), f_serif(18.0), fade(pal.accent, 0.9));
        let nw = ng.size().x;
        p.galley(Pos2::new(x, rect.center().y - ng.size().y * 0.5), ng, pal.text);
        x += nw + 11.0;
    }
    let tc = mix(pal.text, pal.accent, 0.16);
    let tg = p.layout_no_wrap(title.to_owned(), f_serif(15.5), tc);
    let tw = tg.size().x;
    p.galley(Pos2::new(x, rect.center().y - tg.size().y * 0.5), tg, tc);
    // 细线分隔：从标题右侧渐隐到卡片右缘
    let x0 = x + tw + 13.0;
    let x1 = rect.right();
    if x0 < x1 - 10.0 {
        let n = 16;
        for i in 0..n {
            let a = i as f32 / n as f32;
            let b = (i + 1) as f32 / n as f32;
            let lerp = |t: f32| x0 + (x1 - x0) * t;
            p.line_segment(
                [Pos2::new(lerp(a), rect.center().y), Pos2::new(lerp(b), rect.center().y)],
                Stroke::new(1.0_f32, fade(pal.border, 0.95 * (1.0 - a) * (1.0 - a))),
            );
        }
    }
}

/// 不对称网格的一行两列（`ratio` = 左列占比，0~1）。
///
/// 两列各拿一个"零高"的独立 Ui：内容把各自的 `min_rect` 撑开，行高取两者最大值——
/// 于是**允许两列不等宽、不等高**（不对称网格的关键），也允许某列内部再左右不对称。
/// 内容区太窄时自动退化为 50/50，且两列各自不窄于 240px，控件不会被挤出卡片。
fn row2(ui: &mut Ui, ratio: f32, body: impl FnOnce(&mut Ui, &mut Ui)) {
    let avail = ui.available_width().max(200.0);
    let usable = (avail - GUTTER).max(160.0);
    let ratio = if avail < STACK_BELOW { 0.5 } else { ratio.clamp(0.3, 0.75) };
    // 两列各留 240px 下限；窄于 480 时退化为对半开（不 clamp，避免 min>max 直接 panic）
    let (lw, rw) = if usable >= 480.0 {
        let l = (usable * ratio).clamp(240.0, usable - 240.0);
        (l, usable - l)
    } else {
        let l = usable * 0.5;
        (l, usable - l)
    };
    let top = ui.cursor().min;
    let layout = Layout::top_down(Align::Min);
    let mut lui = ui.new_child(
        UiBuilder::new().max_rect(Rect::from_min_size(top, Vec2::new(lw, 0.0))).layout(layout),
    );
    let mut rui = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(Pos2::new(top.x + lw + GUTTER, top.y), Vec2::new(rw, 0.0)))
            .layout(layout),
    );
    body(&mut lui, &mut rui);
    let h = lui.min_rect().height().max(rui.min_rect().height());
    ui.advance_cursor_after_rect(Rect::from_min_size(top, Vec2::new(avail, h)));
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
    /// 当前分区的入场起始时刻（`Context::input().time`）：切换分区时重置，
    /// 卡片入场动画因此**只播一次**，不会被每帧重放
    enter_t0: f64,
    /// 本帧时间（每帧取一次，卡片入场进度共用）
    enter_now: f64,

    // ---- v1.3.1：真透底 + 玻璃厚度 ----
    /// 毛玻璃来源（应用级设置；与 `store.app.glass.mode` 同步，切换时写回并重新申请）
    glass_mode: GlassMode,
    /// 玻璃强度 0.3~1.0（与 `store.app.glass.level` 同步；仅系统模糊生效时影响底色）
    glass_level: f32,
    /// 系统模糊是否生效（每帧读 `windowfx::blur_active()`）→ 决定用真半透明底还是不透明兜底
    blur_active: bool,
    /// `windowfx::apply_once()` 的实际结果（`None` = 三种方式都失败）
    blur_kind: Option<crate::system::windowfx::BlurKind>,
    /// 还在等 `windowfx::apply_once()` 出结果 → 「系统」分区显示「检测中…」
    blur_pending: bool,
    /// 平滑跟随的鼠标位置（`None` = 还没收到鼠标事件，用窗口中心兜底）
    mouse_smooth: Option<Pos2>,
    /// 上一帧时间（鼠标跟随的时间插值用）
    fx_t: f64,
    /// 背板预烘焙贴图（基底 + 渐变 + 网格 + 噪点，一次性栅格化）
    backdrop: Option<egui::TextureHandle>,
    /// 贴图缓存键：(像素宽, 像素高, 深色, 透底, 玻璃强度×100)
    ///
    /// **强度必须在键里**：它决定底色每一层的 alpha，只有重烘焙才会反映到贴图上
    /// （无系统模糊时强度被忽略，末位固定 0，避免白烘）。
    backdrop_key: (usize, usize, bool, bool, u8),
    /// 上次烘焙时刻（拖拽缩放的连续变化按 [`BAKE_DEBOUNCE`] 去抖）
    backdrop_baked_at: Instant,
}

/// 启动设置窗口（独立进程模式：阻塞直到窗口关闭，关闭即进程结束）
pub fn run(store: Arc<Mutex<PresetStore>>, title: &'static str) -> eframe::Result<()> {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size([1040.0, 860.0])
        // 最小宽度要容纳「侧栏 200 + 间距 24 + 不对称两列网格」，再窄控件就会被挤扁
        .with_min_inner_size([980.0, 660.0])
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
        // ---- 字体：内置三套（正文 / 衬线 / 粗体），**只在启动时安装一次** ----
        let fs = fonts::install(&cc.egui_ctx);
        info!("字体安装: sans={} serif={} bold={}", fs.sans, fs.serif, fs.bold);

        let (preset, active_name, lang, theme, glass) = {
            let g = store.lock();
            (g.get_active().clone(), g.active_name(), g.app.lang(), g.app.theme.clone(), g.app.glass)
        };
        // 恢复上次选择的毛玻璃来源（强度不必预置：每帧由 `glass_level` 直接算底色）。
        // 首帧 `update` 里的 `apply_once` 就会按这个模式申请系统模糊。
        crate::system::windowfx::set_mode(glass.mode);
        let dark = match theme.as_str() {
            "light" => false,
            _ => true,
        };
        let pal = if dark { Palette::dark() } else { Palette::light() };

        // ---- 排版节奏（三族分工见文件头）----
        // H1 26~28 serif · H2 15.5 serif · 数值 11.5~14 bold · 正文 12.5 sans · 说明 10.5 dim
        let mut style = (*cc.egui_ctx.style()).clone();
        style.text_styles = [
            (egui::TextStyle::Heading, f_serif(22.0)),
            (egui::TextStyle::Body, f_sans(12.5)),
            (egui::TextStyle::Button, f_sans(12.5)),
            (egui::TextStyle::Small, f_sans(10.5)),
            (egui::TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();
        style.spacing.item_spacing = Vec2::new(10.0, 12.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        // 默认控件的过渡时长与自绘控件对齐（同一条 ease-out 曲线）
        style.animation_time = ANIM_HOVER;
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
            enter_t0: 0.0,
            enter_now: 0.0,
            glass_mode: glass.mode,
            glass_level: glass.level_clamped(),
            blur_active: false,
            blur_kind: None,
            blur_pending: true,
            mouse_smooth: None,
            fx_t: 0.0,
            backdrop: None,
            backdrop_key: (0, 0, false, false, 0),
            backdrop_baked_at: Instant::now(),
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

    /// 入场淡入系数（0.10→1）：卡片内部自绘的内容（预览图等）用它一起渐显
    fn enter_alpha(&self, idx: usize) -> f32 {
        0.10 + 0.90 * ease_enter(self.enter_k(idx))
    }

    /// 第 `idx` 张卡片的入场进度（0→1，线性进度；曲线由调用方按 expo_out 缓动）。
    /// 每张错峰 45ms —— 分区切换时整列卡片"依次落下"，而不是齐刷刷跳出来。
    fn enter_k(&self, idx: usize) -> f32 {
        let t = (self.enter_now - self.enter_t0 - idx as f64 * ENTER_STAGGER as f64) / CARD_ENTER_DUR as f64;
        t.clamp(0.0, 1.0) as f32
    }

    /// 切换分区：重置入场时钟 → 新分区内容淡入 + 轻微上移，禁止硬切
    fn goto_section(&mut self, idx: usize) {
        if self.active_section != idx {
            self.active_section = idx;
            self.enter_t0 = self.enter_now;
            self.recording = None;
        }
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
        // 品牌标记：霓虹紫 → 克莱因蓝渐变方块 + 几何准星 + 一圈紫光
        let mark = Rect::from_center_size(Pos2::new(rect.left() + 20.0, cy), Vec2::splat(34.0));
        let mut shapes = Vec::new();
        push_shadow(&mut shapes, mark, 11.0, pal, 0.55);
        shapes.push(Shape::rect_filled(mark.expand(3.5), Rounding::same(15.0), fade(pal.accent, 0.15)));
        push_vgrad(&mut shapes, mark, 11.0, pal.accent_bright, mix(pal.accent_deep, pal.accent2, 0.55), 6);
        shapes.push(Shape::rect_stroke(mark, Rounding::same(11.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.75))));
        shapes.push(Shape::hline(
            Rangef::new(mark.left() + 8.0, mark.right() - 8.0),
            mark.top() + 1.2,
            Stroke::new(1.0_f32, rgba(255, 255, 255, 120)),
        ));
        p.add(Shape::Vec(shapes));
        let c = mark.center();
        p.circle_stroke(c, 7.4, Stroke::new(1.5_f32, Color32::WHITE));
        for (dx, dy) in [(0.0_f32, -1.0_f32), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
            p.line_segment(
                [Pos2::new(c.x + dx * 5.2, c.y + dy * 5.2), Pos2::new(c.x + dx * 10.4, c.y + dy * 10.4)],
                Stroke::new(1.5_f32, Color32::WHITE),
            );
        }
        // H1：衬线 26
        p.text(
            Pos2::new(mark.right() + 13.0, cy - 9.0),
            Align2::LEFT_CENTER,
            t(lang, "title"),
            f_serif(26.0),
            pal.text,
        );
        // 副标题：版本号走 bold 族（粗细对比）
        p.text(
            Pos2::new(mark.right() + 15.0, cy + 14.0),
            Align2::LEFT_CENTER,
            format!("v{}", crate::VERSION),
            f_bold(10.5),
            fade(pal.dim, 0.95),
        );
        // 标题区底部：一条极细的镜面分隔线（不靠粗边框划分区域）
        p.line_segment(
            [Pos2::new(rect.left() + 4.0, rect.bottom() + 6.0), Pos2::new(rect.right() - 4.0, rect.bottom() + 6.0)],
            Stroke::new(1.0_f32, fade(pal.glass_hi, 0.11)),
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

    /// 左侧导航：玻璃轨 + 选中胶囊（克莱因蓝→霓虹紫）+ accent 光条 + 衬线序号
    fn nav_ui(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        ui.add_space(2.0);
        for (idx, key) in NAV_ITEMS {
            let selected = self.active_section == idx;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
            let hot = anim_bool(ui.ctx(), Id::new(("nav_h", idx)), resp.hovered(), ANIM_HOVER);
            let sel = anim01(ui.ctx(), Id::new(("nav_s", idx)), selected, ANIM_VALUE);
            // hover 抬升与选中态共用同一条曲线：位移 / 底色 / 描边同步过渡
            let lift = HOVER_LIFT * hot * (1.0 - sel);
            let r = Rect::from_min_max(
                Pos2::new(rect.left() + 2.0, rect.top() + 2.0 - lift),
                Pos2::new(rect.right() - 4.0, rect.bottom() - 2.0 - lift),
            );
            let p = ui.painter();
            if sel > 0.01 {
                p.rect_filled(r, Rounding::same(12.0), fade(mix(pal.accent2, pal.accent, 0.72), 0.24 * sel));
                p.rect_stroke(r, Rounding::same(12.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.32 * sel)));
                let bar = Rect::from_min_max(
                    Pos2::new(r.left() + 3.0, r.center().y - 9.0 * sel),
                    Pos2::new(r.left() + 5.4, r.center().y + 9.0 * sel),
                );
                p.rect_filled(bar.expand(2.6), Rounding::same(3.0), fade(pal.accent, 0.20 * sel));
                p.rect_filled(bar, Rounding::same(1.5), fade(pal.accent_bright, sel));
            } else if hot > 0.01 {
                p.rect_filled(r, Rounding::same(12.0), fade(pal.control, 0.85 * hot));
                p.rect_stroke(r, Rounding::same(12.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.10 * hot)));
            }
            // 前景色 / 图标 / 序号都跟着 sel 与 hot 连续过渡（选中不再"啪"地换色）
            let fg = mix(pal.nav_fg, pal.nav_fg_on, hot.max(sel));
            nav_glyph(p, idx, Pos2::new(r.left() + 24.0, r.center().y), mix(fg, pal.accent_bright, sel));
            // 标签：选中项走 bold 族（粗细对比），其余常规无衬线
            let lf = if selected { f_bold(13.0) } else { f_sans(13.0) };
            p.text(Pos2::new(r.left() + 40.0, r.center().y - 0.5), Align2::LEFT_CENTER, t(lang, key), lf, fg);
            // 右侧衬线序号（01…08）
            p.text(
                Pos2::new(r.right() - 12.0, r.center().y - 0.5),
                Align2::RIGHT_CENTER,
                format!("{:02}", idx + 1),
                f_serif(11.0),
                mix(fade(pal.dim, 0.65 + 0.35 * hot), fade(pal.accent_bright, 0.95), sel),
            );
            if resp.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            if resp.clicked() {
                self.goto_section(idx);
            }
        }
        // 导航轨收尾：一条渐隐细线
        ui.add_space(12.0);
        let (lr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
        ui.painter().line_segment(
            [Pos2::new(lr.left() + 10.0, lr.center().y), Pos2::new(lr.right() - 10.0, lr.center().y)],
            Stroke::new(1.0_f32, fade(pal.border, 0.7)),
        );
    }

    /// 底部操作条：连接状态（几何圆点）+ flash 淡出 + 应用 / 退出主程序
    fn bottom_bar(&mut self, ui: &mut Ui, pal: &Palette, rect: Rect) {
        let lang = self.lang;
        let mut shapes = Vec::new();
        push_panel(&mut shapes, rect, R_CARD, pal, 0.0);
        ui.painter().add(Shape::Vec(shapes));

        let inner = rect.shrink2(Vec2::new(24.0, 12.0));
        let mut bui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
        bui.spacing_mut().item_spacing = Vec2::new(10.0, 4.0);

        let (dot, msg, msg_color) = if self.backend_ok {
            if self.dirty {
                (pal.warn, t(lang, "backend_dirty"), pal.warn)
            } else {
                (pal.ok, t(lang, "backend_connected"), pal.label)
            }
        } else {
            (pal.warn, t(lang, "backend_file_mode"), pal.label)
        };
        let (dr, _) = bui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        let con = anim_bool(bui.ctx(), Id::new("conn_ok"), self.backend_ok, ANIM_VALUE);
        let p = bui.painter();
        // 状态点：外圈柔光随连接状态平滑过渡，不闪
        p.circle_filled(dr.center(), 6.4, fade(dot, 0.14 + 0.08 * con));
        p.circle_filled(dr.center(), 3.4, dot);
        bui.label(RichText::new(msg).size(11.5).color(msg_color));

        let mut expire = false;
        bui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button_sized(ui, pal, "apply", t(lang, "push_apply"), Btn::Primary, 176.0, 32.0, 13.5).clicked() {
                self.push_to_backend();
            }
            if button_sized(ui, pal, "quit_app", t(lang, "quit_backend"), Btn::Glass, 108.0, 32.0, 12.5).clicked() {
                if let Some(appdata) = crate::appdata_dir().ok() {
                    let _ = std::fs::write(appdata.join("cmd.json"), "{\"cmd\":\"quit\"}");
                    self.flash(t(lang, "quit_backend_sent").to_string());
                }
            }
            ui.add_space(6.0);
            if let Some((text, at)) = self.status.as_ref() {
                let el = at.elapsed().as_secs_f32();
                let ttl = STATUS_TTL.as_secs_f32();
                if el < ttl {
                    // 淡入 0.09s / 淡出 0.55s，都走 cubic_out——
                    // 原来的线性斜线在末尾是"啪"地暗掉，看着很生硬
                    let k = if el < FLASH_IN {
                        ease_out(el / FLASH_IN)
                    } else if el > ttl - FLASH_OUT {
                        1.0 - ease_out((el - (ttl - FLASH_OUT)) / FLASH_OUT)
                    } else {
                        1.0
                    };
                    // flash 提示走 bold 族：和正文拉开粗细层次
                    ui.label(RichText::new(text.clone()).font(f_bold(11.5)).color(with_alpha(pal.ok, (255.0 * k) as u8)));
                    // flash 存活期间持续重绘：否则淡出要等下一次鼠标事件才会开始（提示会一直亮着），
                    // 到 TTL 也才能在**同一帧**消失。1.9s 后就停，静止时依旧不空转。
                    ui.ctx().request_repaint();
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
        // ---- 01 主视觉（跨 2 列）：左 62% 实时预览 / 右 38% 当前参数读数 ----
        card(ui, pal, "hero", "", self.enter_k(0), None, |ui, pal| {
            row2(ui, 0.62, |lu, ru| {
                let h = 238.0;
                let (rect, _) = lu.allocate_exact_size(Vec2::new(lu.available_width(), h), Sense::hover());
                preview::paint_preview(lu, rect, &self.preset, lang, self.enter_alpha(0));
                let (cap, _) = lu.allocate_exact_size(Vec2::new(lu.available_width(), 20.0), Sense::hover());
                lu.painter().text(
                    cap.left_center(),
                    Align2::LEFT_CENTER,
                    t(lang, "preview"),
                    f_sans(10.5),
                    pal.dim,
                );
                lu.painter().text(
                    cap.right_center(),
                    Align2::RIGHT_CENTER,
                    format!("{}×{}", lu.available_width() as i32, h as i32),
                    f_bold(10.5),
                    fade(pal.dim, 0.9),
                );

                // 右列：读数（形状名走衬线、数值走 bold —— 粗细对比最明显的地方）
                ru.add_space(6.0);
                ru.label(
                    RichText::new(shape_name(lang, self.preset.shape))
                        .font(f_serif(21.0))
                        .color(pal.text),
                );
                ru.add_space(10.0);
                for (k, v) in [
                    (t(lang, "size"), format!("{:.0}", self.preset.size)),
                    (t(lang, "thickness"), format!("{:.1}", self.preset.thickness)),
                    (t(lang, "opacity"), format!("{:.2}", self.preset.opacity)),
                ] {
                    let (rr, _) = ru.allocate_exact_size(Vec2::new(ru.available_width(), 26.0), Sense::hover());
                    let p = ru.painter();
                    p.text(rr.left_center(), Align2::LEFT_CENTER, k, f_sans(11.5), pal.dim);
                    p.text(rr.right_center(), Align2::RIGHT_CENTER, v, f_bold(14.0), pal.text);
                }
                ru.add_space(8.0);
                // 颜色条：单色模式一块，多色模式四象限并列
                let swatches: Vec<String> = if self.preset.multicolor {
                    vec![
                        self.preset.colors.top.clone(),
                        self.preset.colors.bottom.clone(),
                        self.preset.colors.left.clone(),
                        self.preset.colors.right.clone(),
                    ]
                } else {
                    vec![self.preset.color.clone()]
                };
                let (sr, _) = ru.allocate_exact_size(Vec2::new(ru.available_width(), 26.0), Sense::hover());
                let p = ru.painter();
                let n = swatches.len() as f32;
                let gap = 8.0;
                let w = ((sr.width() - gap * (n - 1.0)) / n).max(14.0);
                for (i, hex) in swatches.iter().enumerate() {
                    let (r, g, b) = crate::overlay::parse_hex(hex);
                    let cell = Rect::from_min_size(
                        Pos2::new(sr.left() + i as f32 * (w + gap), sr.top() + 3.0),
                        Vec2::new(w, 20.0),
                    );
                    p.rect_filled(cell, Rounding::same(6.0), Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
                    p.rect_stroke(cell, Rounding::same(6.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.55)));
                }
            });
        });

        // ---- 02 形状与样式（窄栏）/ 03 颜色与描边（宽栏）：1+1 的不对称一行 ----
        row2(ui, 0.58, |lu, ru| {
            card(lu, pal, "s_shape", "01/03", self.enter_k(1), Some(t(lang, "shape_style")), |ui, pal| {
                ui.horizontal(|ui| {
                    field_label(ui, pal, t(lang, "shape"));
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
                let has_gap = matches!(
                    self.preset.shape,
                    CrossShape::HollowCross | CrossShape::HollowSquare | CrossShape::HollowCrossDot | CrossShape::GapHair
                );
                if has_gap {
                    ui.add_space(4.0);
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
            });

            card(ru, pal, "s_color", "02/03", self.enter_k(2), Some(t(lang, "color_outline")), |ui, pal| {
                if checkbox(ui, pal, "multicolor", &mut self.preset.multicolor, t(lang, "multicolor")) {
                    self.dirty = true;
                }
                ui.add_space(4.0);
                if self.preset.multicolor {
                    if color_row_ui(ui, pal, t(lang, "color_top"), &mut self.hex_top, &mut self.preset.colors.top) { self.dirty = true; }
                    if color_row_ui(ui, pal, t(lang, "color_bottom"), &mut self.hex_bottom, &mut self.preset.colors.bottom) { self.dirty = true; }
                    if color_row_ui(ui, pal, t(lang, "color_left"), &mut self.hex_left, &mut self.preset.colors.left) { self.dirty = true; }
                    if color_row_ui(ui, pal, t(lang, "color_right"), &mut self.hex_right, &mut self.preset.colors.right) { self.dirty = true; }
                } else if color_row_ui(ui, pal, t(lang, "main_color"), &mut self.hex_main, &mut self.preset.color) {
                    self.dirty = true;
                }
                ui.add_space(8.0);
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
        });

        // ---- 04 模板画廊（跨 2 列）----
        card(ui, pal, "s_tpl", "03/03", self.enter_k(3), Some(t(lang, "templates")), |ui, pal| {
            note(ui, pal, t(lang, "templates_hint"));
            ui.add_space(6.0);
            let cols = 4usize;
            let gap = 14.0;
            let avail = ui.available_width();
            let cell_w = ((avail - gap * (cols as f32 - 1.0)) / cols as f32).floor().max(90.0);
            for row in 0..TEMPLATES.len().div_ceil(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for col in 0..cols {
                        let i = row * cols + col;
                        if i >= TEMPLATES.len() {
                            break;
                        }
                        let (key, _) = TEMPLATES[i];
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(cell_w, 104.0), Sense::click());
                        let hot = anim_bool(ui.ctx(), Id::new(("tpl", i)), resp.hovered(), ANIM_HOVER);
                        let c = rect.translate(Vec2::new(0.0, -3.0 * hot));
                        let mut shapes = Vec::new();
                        push_panel(&mut shapes, c, 14.0, pal, hot);
                        ui.painter().add(Shape::Vec(shapes));
                        let prev = Rect::from_min_max(Pos2::new(c.left() + 6.0, c.top() + 6.0), Pos2::new(c.right() - 6.0, c.top() + 66.0));
                        preview::paint_preview_cells(ui, prev, &self.tpl_previews[i], lang, 16.0, self.enter_alpha(3));
                        ui.painter().text(
                            Pos2::new(c.center().x, c.bottom() - 18.0),
                            Align2::CENTER_CENTER,
                            ellipsize(t(lang, key), 12),
                            if hot > 0.5 { f_bold(11.0) } else { f_sans(11.0) },
                            mix(pal.label, pal.text, hot),
                        );
                        if resp.clicked() {
                            (TEMPLATES[i].1)(&mut self.preset);
                            self.sync_buffers();
                            self.dirty = true;
                            self.flash(t(lang, "template_applied").to_string());
                        }
                        if resp.hovered() {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                        }
                    }
                });
            }
        });
    }

    fn section_dynamic(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        // ---- 05 动态准星：左 60% 参数 / 右 40% 开火扩散后的形态预览 ----
        card(ui, pal, "dynamic", "", self.enter_k(0), Some(t(lang, "dynamic")), |ui, pal| {
            row2(ui, 0.60, |lu, ru| {
                if slider_row(lu, pal, t(lang, "fire_expand"), |ui, w| {
                    slider_f32(ui, pal, "fe", &mut self.preset.dynamic.fire_expand_px, Rangef::new(0.0, 80.0), w, 0, "px")
                }) {
                    self.dirty = true;
                }
                let mut rec = self.preset.dynamic.recover_ms as i32;
                if slider_row(lu, pal, t(lang, "recover_ms"), |ui, w| {
                    slider_i32(ui, pal, "rm", &mut rec, Rangef::new(20.0, 500.0), w)
                }) {
                    self.preset.dynamic.recover_ms = rec.max(1) as u32;
                    self.dirty = true;
                }
                if checkbox(lu, pal, "recoil", &mut self.preset.dynamic.recoil_indicator, t(lang, "recoil_indicator")) {
                    self.dirty = true;
                }

                // 右列：把「开火扩散量」叠加到几何上的形态（复用主预览绘制，不新增逻辑）
                let (pr, _) = ru.allocate_exact_size(Vec2::new(ru.available_width(), 152.0), Sense::hover());
                preview::paint_preview_expanded(ru, pr, &self.preset, lang, self.preset.dynamic.fire_expand_px, self.enter_alpha(0));
                let (cap, _) = ru.allocate_exact_size(Vec2::new(ru.available_width(), 22.0), Sense::hover());
                let p = ru.painter();
                p.text(cap.left_center(), Align2::LEFT_CENTER, t(lang, "fire_expand"), f_sans(10.5), pal.dim);
                p.text(
                    cap.right_center(),
                    Align2::RIGHT_CENTER,
                    format!("+{:.0}px", self.preset.dynamic.fire_expand_px),
                    f_bold(13.0),
                    mix(pal.text, pal.accent_bright, 0.25),
                );
            });
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
        // ---- 06 位置：左 56% 显示器列表 / 右 44% 坐标与吸附 ----
        card(ui, pal, "position", "", self.enter_k(0), Some(t(lang, "position")), |ui, pal| {
            row2(ui, 0.56, |lu, ru| {
                // 左列：显示器列表
                lu.horizontal(|ui| {
                    field_label(ui, pal, t(lang, "monitors_title"));
                    if button_sized(ui, pal, "mon_refresh", t(lang, "monitor_refresh"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        self.monitors = crate::system::monitor::monitors();
                        self.monitors_at = Some(Instant::now());
                    }
                });
                // 跟随前台窗口
                if monitor_row(lu, pal, self.preset.position.monitor == -1, t(lang, "monitor_follow"), None) {
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
                    if monitor_row(lu, pal, self.preset.position.monitor == i as i32, &label, Some(m)) {
                        self.preset.position.monitor = i as i32;
                        self.dirty = true;
                    }
                }

                // 右列：坐标 + 居中 + 吸附（窄栏里每项各占一行，控件不会被挤扁）
                let mut x = match self.preset.position.x {
                    PosVal::Px(v) => v as i32,
                    PosVal::Center => 0,
                };
                let mut y = match self.preset.position.y {
                    PosVal::Px(v) => v as i32,
                    PosVal::Center => 0,
                };
                let mut moved = false;
                if coord_row(ru, pal, t(lang, "pos_x"), &mut x) {
                    moved = true;
                }
                if coord_row(ru, pal, t(lang, "pos_y"), &mut y) {
                    moved = true;
                }
                if moved {
                    self.preset.position.x = PosVal::Px(x as f32);
                    self.preset.position.y = PosVal::Px(y as f32);
                    self.dirty = true;
                }
                ru.add_space(6.0);
                if button_sized(ru, pal, "mon_center", t(lang, "center_on_monitor"), Btn::Glass, 0.0, 28.0, 12.0).clicked() {
                    // 用相对该屏的像素值锁死中心（Center 会跟随主屏/前台窗口，这里显式写点）
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
                    self.preset.position.x = PosVal::Px(target.0 as f32);
                    self.preset.position.y = PosVal::Px(target.1 as f32);
                    self.dirty = true;
                    self.flash(t(lang, "center_on_monitor").to_string());
                }
                if button_sized(ru, pal, "mon_reset", t(lang, "center_btn"), Btn::Ghost, 0.0, 28.0, 12.0).clicked() {
                    self.preset.position.x = PosVal::Center;
                    self.preset.position.y = PosVal::Center;
                    self.dirty = true;
                }
                ru.add_space(8.0);
                if checkbox(ru, pal, "snap", &mut self.preset.snap_to_window, t(lang, "snap_to_window")) {
                    self.dirty = true;
                }
                ru.add_space(4.0);
                note(ru, pal, t(lang, "snap_note"));
            });
        });
    }

    fn section_gamepad(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        // ---- 07 手柄：左 58% 参数 / 右 42% 手柄图示 + 说明 ----
        card(ui, pal, "gamepad", "", self.enter_k(0), Some(t(lang, "gamepad")), |ui, pal| {
            row2(ui, 0.58, |lu, ru| {
                lu.horizontal(|ui| {
                    field_label(ui, pal, t(lang, "ads_mode"));
                    let mut picked: Option<AdsMode> = None;
                    combo(ui, 150.0, "ads_mode_glass", ads_mode_name(lang, self.preset.gamepad.ads_mode).to_string(), |ui| {
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
                lu.horizontal(|ui| {
                    field_label(ui, pal, t(lang, "ads_button"));
                    let cur = match self.preset.gamepad.ads_button {
                        AdsButton::LeftTrigger => t(lang, "ads_left_trigger"),
                        AdsButton::RightTrigger => t(lang, "ads_right_trigger"),
                        AdsButton::LeftBumper => t(lang, "ads_left_bumper"),
                        AdsButton::RightBumper => t(lang, "ads_right_bumper"),
                    };
                    let mut picked: Option<AdsButton> = None;
                    let w = (ui.available_width() - 4.0).min(176.0);
                    combo(ui, w.max(140.0), "ads_btn_glass", cur.to_string(), |ui| {
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
                if slider_row(lu, pal, t(lang, "trigger_threshold"), |ui, w| {
                    slider_i32(ui, pal, "thr", &mut thr, Rangef::new(0.0, 255.0), w)
                }) {
                    self.preset.gamepad.trigger_threshold = thr.clamp(0, 255) as u8;
                    self.dirty = true;
                }
                if checkbox(lu, pal, "gp_fire", &mut self.preset.gamepad.fire_expand, t(lang, "gamepad_fire_expand")) {
                    self.dirty = true;
                }

                // 右列：手柄图示（几何绘制）+ 说明（窄栏里折行更好读）
                let (gr, _) = ru.allocate_exact_size(Vec2::new(ru.available_width(), 74.0), Sense::hover());
                gamepad_glyph(ru.painter(), gr.center(), pal);
                ru.add_space(6.0);
                note(ru, pal, t(lang, "gamepad_note"));
            });
        });
    }

    fn section_hotkey(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        // ---- 08 热键（宽栏）/ 09 右键（窄栏）----
        row2(ui, 0.60, |lu, ru| {
            card(lu, pal, "hotkey", "01/02", self.enter_k(0), Some(t(lang, "hotkey")), |ui, pal| {
                let toggle = self.preset.hotkey_toggle;
                self.hotkey_field(ui, pal, 0, t(lang, "hotkey_toggle"), toggle);
                let next = self.preset.hotkey_next_profile;
                self.hotkey_field(ui, pal, 1, t(lang, "hotkey_next"), next);
                ui.add_space(4.0);
                note(ui, pal, t(lang, "hotkey_note"));
            });

            card(ru, pal, "rclick", "02/02", self.enter_k(1), Some(t(lang, "right_click")), |ui, pal| {
                if checkbox(ui, pal, "rc_toggle", &mut self.preset.right_click_toggle, t(lang, "right_click")) {
                    self.dirty = true;
                }
                if self.preset.right_click_toggle {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        field_label(ui, pal, t(lang, "right_click_mode"));
                        let cur = match self.preset.right_click_mode {
                            RightClickMode::Click => t(lang, "rc_click"),
                            RightClickMode::HoldShow => t(lang, "rc_hold_show"),
                            RightClickMode::HoldHide => t(lang, "rc_hold_hide"),
                        };
                        let mut picked: Option<RightClickMode> = None;
                        let w = (ui.available_width() - 4.0).min(150.0);
                        combo(ui, w.max(120.0), "rc_mode_glass", cur.to_string(), |ui| {
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
        });
    }

    /// 单个热键：录制框 + 清空 + 手工输入（保留旧版直接输入字符串的能力）
    fn hotkey_field(&mut self, ui: &mut Ui, pal: &Palette, slot: usize, label: &str, hk: Hotkey) {
        let lang = self.lang;
        let recording = self.recording == Some(slot);
        ui.horizontal(|ui| {
            field_label(ui, pal, label);
            let avail = ui.available_width();
            let text = if recording {
                t(lang, "hotkey_recording").to_string()
            } else if hk.is_empty() {
                t(lang, "hotkey_record").to_string()
            } else {
                hk.to_string()
            };
            let resp = key_box(ui, pal, slot, &text, (avail - 96.0).max(118.0), recording, hk.is_empty() && !recording);
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
            field_label(ui, pal, t(lang, "hotkey_manual"));
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

        // ---- 10 自定义图片：左 46% 缩略图井 / 右 54% 路径 + 缩放 ----
        card(ui, pal, "image", "", self.enter_k(0), Some(t(lang, "custom_image")), |ui, pal| {
            row2(ui, 0.46, |lu, ru| {
                // 左列：缩略图井（居中，带玻璃厚度）
                let (slot, _) = lu.allocate_exact_size(Vec2::new(lu.available_width(), 122.0), Sense::hover());
                let well = Rect::from_center_size(slot.center(), Vec2::new(150.0, 118.0));
                let mut shapes = Vec::new();
                push_well(&mut shapes, well, 14.0, pal);
                lu.painter().add(Shape::Vec(shapes));
                let inner = well.shrink(7.0);
                match &self.thumb {
                    Thumb::Ready(tex) => {
                        lu.painter().image(
                            tex.id(),
                            inner,
                            Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    Thumb::Failed => {
                        lu.painter().text(inner.center(), Align2::CENTER_CENTER, t(lang, "image_unreadable"), f_sans(11.0), pal.danger);
                    }
                    Thumb::Empty => {
                        lu.painter().text(inner.center(), Align2::CENTER_CENTER, t(lang, "image_preview"), f_sans(11.0), pal.dim);
                    }
                }

                // 右列：路径 + 选择 / 清除 + 缩放
                ru.label(RichText::new(t(lang, "image_path")).size(10.5).color(pal.dim));
                let text = if self.preset.image.path.is_empty() {
                    t(lang, "image_none").to_string()
                } else {
                    file_name(&self.preset.image.path)
                };
                let (rect, resp) = ru.allocate_exact_size(Vec2::new(ru.available_width(), 30.0), Sense::hover());
                let mut shapes = Vec::new();
                push_well(&mut shapes, rect, 9.0, pal);
                ru.painter().add(Shape::Vec(shapes));
                ru.painter().text(
                    Pos2::new(rect.left() + 10.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    ellipsize(&text, 40),
                    f_sans(11.5),
                    if self.preset.image.path.is_empty() { pal.dim } else { pal.text },
                );
                let tip = self.preset.image.path.clone();
                if !tip.is_empty() {
                    resp.on_hover_text(tip);
                }
                ru.add_space(4.0);
                ru.horizontal(|ui| {
                    if button_sized(ui, pal, "img_pick", t(lang, "image_choose"), Btn::Glass, 0.0, 28.0, 11.5).clicked() {
                        if let Some(p) = crate::system::filedialog::pick_image() {
                            self.preset.image.path = p.to_string_lossy().into_owned();
                            self.dirty = true;
                        }
                    }
                    if button_sized(ui, pal, "img_clear", t(lang, "clear"), Btn::Ghost, 0.0, 28.0, 11.5).clicked() {
                        self.preset.image.path.clear();
                        self.dirty = true;
                    }
                });
                ru.add_space(8.0);
                if slider_row(ru, pal, t(lang, "image_scale"), |ui, w| {
                    slider_f32(ui, pal, "isc", &mut self.preset.image.scale, Rangef::new(0.1, 5.0), w, 2, "x")
                }) {
                    self.dirty = true;
                }
                if self.preset.image.path.is_empty() {
                    ru.add_space(2.0);
                    note(ru, pal, t(lang, "image_none"));
                }
            });
        });
    }

    fn section_presets(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        let names = { self.store.lock().preset_names() };

        card(ui, pal, "presets", "01/02", self.enter_k(0), Some(t(lang, "presets")), |ui, pal| {
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
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
                let hot = anim_bool(ui.ctx(), Id::new(("prow", name)), resp.hovered(), ANIM_HOVER);
                // 激活态也走缓动：点「启用」时底色 / 光条 / 圆点 / 文字一起过渡，不"啪"地跳
                let act = anim01(ui.ctx(), Id::new(("prow_a", name)), active, ANIM_VALUE);
                let lift = HOVER_LIFT * hot * (1.0 - act);
                let r = Rect::from_min_max(
                    Pos2::new(rect.left(), rect.top() + 1.0 - lift),
                    Pos2::new(rect.right(), rect.bottom() - 1.0 - lift),
                );
                let p = ui.painter();
                if hot > 0.01 || act > 0.01 {
                    p.rect_filled(
                        r,
                        Rounding::same(12.0),
                        mix(fade(pal.control, hot * 0.9), fade(mix(pal.accent2, pal.accent, 0.75), 0.22), act),
                    );
                    p.rect_stroke(
                        r,
                        Rounding::same(12.0),
                        Stroke::new(1.0_f32, fade(pal.glass_hi, (0.08 * hot).max(0.26 * act))),
                    );
                    // 左侧激活光条：高度与不透明度一起长出来
                    p.rect_filled(
                        Rect::from_min_max(
                            Pos2::new(r.left() + 3.0, r.center().y - 9.0 * act),
                            Pos2::new(r.left() + 5.4, r.center().y + 9.0 * act),
                        ),
                        Rounding::same(1.5),
                        fade(pal.accent_bright, act),
                    );
                }
                p.circle_filled(Pos2::new(r.left() + 19.0, r.center().y), 3.2, mix(fade(pal.dim, 0.9), pal.ok, act));
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
                    if active { f_bold(12.5) } else { f_sans(12.5) },
                    mix(mix(pal.label, pal.text, hot), pal.text, act),
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

        // ---- 12 游戏绑定：左 60% 已有绑定 / 右 40% 添加表单（表单纵向排布，窄栏不挤）----
        card(ui, pal, "bindings", "02/02", self.enter_k(1), Some(t(lang, "bindings")), |ui, pal| {
            let bindings: Vec<(String, String)> = {
                let store = self.store.lock();
                store.app.game_bindings.iter().map(|b| (b.exe.clone(), b.preset.clone())).collect()
            };
            let mut remove: Option<usize> = None;
            row2(ui, 0.60, |lu, ru| {
                // 左列：已有绑定
                if bindings.is_empty() {
                    lu.label(RichText::new(t(lang, "binding_none")).size(11.0).color(pal.dim));
                }
                for (i, (exe, preset)) in bindings.iter().enumerate() {
                    let (rect, _) = lu.allocate_exact_size(Vec2::new(lu.available_width(), 30.0), Sense::hover());
                    lu.painter().rect_filled(rect, Rounding::same(10.0), fade(pal.control, 0.55));
                    lu.painter().rect_stroke(rect, Rounding::same(10.0), Stroke::new(1.0_f32, pal.border));
                    lu.painter().text(Pos2::new(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, exe, f_sans(11.5), pal.text);
                    lu.painter().text(Pos2::new(rect.right() - 84.0, rect.center().y), Align2::RIGHT_CENTER, preset, f_bold(11.5), pal.accent_bright);
                    let brect = Rect::from_min_max(Pos2::new(rect.right() - 76.0, rect.top() + 3.0), Pos2::new(rect.right() - 6.0, rect.bottom() - 3.0));
                    let mut bui = lu.new_child(UiBuilder::new().max_rect(brect).layout(Layout::right_to_left(Align::Center)));
                    if button_sized(&mut bui, pal, &format!("b_rm{i}"), t(lang, "binding_remove"), Btn::Ghost, 0.0, 22.0, 11.0).clicked() {
                        remove = Some(i);
                    }
                }

                // 右列：添加表单（输入框 / 预设下拉 / 按钮纵向排布）
                let tw = ru.available_width() - 2.0;
                text_field(ru, pal, &mut self.new_binding_exe, tw, t(lang, "binding_exe"));
                ru.add_space(6.0);
                let names = { self.store.lock().preset_names() };
                let mut picked: Option<String> = None;
                let pw = ru.available_width() - 2.0;
                combo(ru, pw, "bind_preset_glass", self.new_binding_preset.clone(), |ui| {
                    for n in &names {
                        if ui.selectable_label(&self.new_binding_preset == n, n.clone()).clicked() {
                            picked = Some(n.clone());
                        }
                    }
                });
                if let Some(n) = picked {
                    self.new_binding_preset = n;
                }
                ru.add_space(6.0);
                if button_sized(ru, pal, "b_add", t(lang, "binding_add"), Btn::Glass, 0.0, 28.0, 12.0).clicked() {
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
                ru.add_space(10.0);
                note(ru, pal, t(lang, "binding_note"));
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
        });
    }

    /// 系统：应用级开关（开机自启）
    fn section_system(&mut self, ui: &mut Ui, pal: &Palette) {
        let lang = self.lang;
        // ---- 13 系统：左 60% 开机自启 / 右 40% 说明 ----
        card(ui, pal, "system", "", self.enter_k(0), Some(t(lang, "system")), |ui, pal| {
            row2(ui, 0.60, |lu, ru| {
                let mut autostart = { self.store.lock().app.autostart };
                if checkbox(lu, pal, "autostart", &mut autostart, t(lang, "autostart")) {
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

                ru.add_space(4.0);
                note(ru, pal, t(lang, "autostart_note"));
            });

            // ---- 毛玻璃状态：用户不用翻日志就知道系统模糊有没有生效 ----
            // 生效 → 窗口用真半透明底（桌面透得进来）；不可用 → 已自动改用不透明底兜底
            ui.add_space(12.0);
            let (dot, key) = if self.blur_pending {
                (pal.warn, "glass_checking")
            } else {
                match self.blur_kind {
                    Some(crate::system::windowfx::BlurKind::Acrylic) => (pal.ok, "glass_acrylic"),
                    Some(crate::system::windowfx::BlurKind::Backdrop) => (pal.ok, "glass_backdrop"),
                    Some(crate::system::windowfx::BlurKind::Aero) => (pal.ok, "glass_aero"),
                    _ => (pal.danger, "glass_unavailable"),
                }
            };
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.4, dot);
                ui.label(
                    RichText::new(format!("{}：{}", t(lang, "glass"), t(lang, key)))
                        .size(11.0)
                        .color(mix(pal.label, pal.text, 0.30)),
                );
            });
            note(ui, pal, t(lang, "glass_note"));

            // ---- 毛玻璃模式：不同系统能用的模糊方式不同（亚克力 / Win11 背板），
            // 观感差异只有真机看得出来，所以给用户手动切换、实时对比 ----
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                field_label(ui, pal, t(lang, "glass_mode"));
                let w = ui.available_width() - 4.0;
                let cur = glass_mode_name(lang, self.glass_mode);
                let mut picked: Option<GlassMode> = None;
                combo(ui, w.max(140.0), "glass_mode", cur.to_string(), |ui| {
                    for m in [GlassMode::Auto, GlassMode::Acrylic, GlassMode::Backdrop, GlassMode::Off] {
                        if ui.selectable_label(self.glass_mode == m, glass_mode_name(lang, m)).clicked() {
                            picked = Some(m);
                        }
                    }
                });
                if let Some(m) = picked {
                    if m != self.glass_mode {
                        self.glass_mode = m;
                        {
                            let mut store = self.store.lock();
                            store.app.glass.mode = m;
                            let _ = store.save_app();
                        }
                        // 模式必须重新申请系统模糊才立刻生效；强度不用（只改底色 → 重烘焙贴图）
                        crate::system::windowfx::set_mode(m);
                        self.blur_kind = match crate::system::windowfx::reapply() {
                            Some(crate::system::windowfx::BlurKind::None) | None => None,
                            k => k,
                        };
                        self.blur_pending = false;
                        self.blur_active = crate::system::windowfx::blur_active();
                        self.flash(t(lang, "saved").to_string());
                    }
                }
            });
            // ---- 玻璃强度：越大越透（仅系统模糊生效时有效，无模糊时忽略）----
            let mut lvl = self.glass_level;
            if slider_row(ui, pal, t(lang, "glass_level"), |ui, w| {
                slider_f32(ui, pal, "glass_level", &mut lvl, Rangef::new(GLASS_MIN, GLASS_MAX), w, 2, "")
            }) {
                // 滑杆连续取值 → 量化到 0.05 步进并消掉浮点尾数（显示两位小数）；
                // 只有真正跨过一个步进才落库，拖动时不会每帧写盘
                let q = (((lvl / GLASS_STEP).round() * GLASS_STEP) * 100.0).round() / 100.0;
                let q = q.clamp(GLASS_MIN, GLASS_MAX);
                if (q - self.glass_level).abs() > f32::EPSILON {
                    self.glass_level = q;
                    let mut store = self.store.lock();
                    store.app.glass.level = q;
                    let _ = store.save_app();
                }
            }
            note(ui, pal, t(lang, "glass_level_hint"));
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

/// 滑杆行：标签列（自适应宽）+ 占满剩余宽度的滑杆
fn slider_row(ui: &mut Ui, pal: &Palette, label: &str, body: impl FnOnce(&mut Ui, f32) -> bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        field_label(ui, pal, label);
        changed = body(ui, ui.available_width() - 4.0);
    });
    changed
}

/// 坐标行：标签 + DragValue（窄栏里每项独占一行也放得下）
fn coord_row(ui: &mut Ui, pal: &Palette, label: &str, v: &mut i32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        field_label(ui, pal, label);
        ui.scope(|ui| {
            apply_style(ui, pal);
            let w = ui.available_width() - 2.0;
            if ui.add_sized(Vec2::new(w.max(60.0), 26.0), egui::DragValue::new(v).speed(2.0)).changed() {
                changed = true;
            }
        });
    });
    changed
}

/// 手柄图形（纯几何：机身 + 双摇杆 + 肩键），只用来看，不带交互
fn gamepad_glyph(p: &egui::Painter, c: Pos2, pal: &Palette) {
    let body = Rect::from_center_size(c, Vec2::new(96.0, 48.0));
    let line = Stroke::new(1.3_f32, fade(pal.label, 0.85));
    let soft = Stroke::new(1.3_f32, fade(pal.dim, 0.7));
    p.rect_filled(body.expand(6.0), Rounding::same(20.0), fade(pal.accent, 0.06));
    p.rect_stroke(body, Rounding::same(16.0), line);
    // 肩键
    p.line_segment([body.left_top() + Vec2::new(8.0, -6.0), body.left_top() + Vec2::new(30.0, -6.0)], soft);
    p.line_segment([body.right_top() + Vec2::new(-30.0, -6.0), body.right_top() + Vec2::new(-8.0, -6.0)], soft);
    // 双摇杆
    let dl = Pos2::new(body.left() + 26.0, body.center().y);
    p.circle_stroke(dl, 8.0, line);
    p.circle_filled(dl, 3.0, fade(pal.accent_bright, 0.9));
    let dr = Pos2::new(body.right() - 26.0, body.center().y);
    p.circle_stroke(dr, 8.0, line);
    p.circle_filled(dr, 3.0, fade(pal.accent_bright, 0.9));
    // 中央装饰线
    p.line_segment(
        [Pos2::new(body.center().x - 10.0, body.center().y), Pos2::new(body.center().x + 10.0, body.center().y)],
        Stroke::new(1.0_f32, fade(pal.border, 0.9)),
    );
}

/// 显示器行：玻璃行 + 选中态（克莱因蓝→紫）+ 主屏标记
fn monitor_row(ui: &mut Ui, pal: &Palette, selected: bool, label: &str, mon: Option<&MonitorInfo>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click());
    let hot = anim_bool(ui.ctx(), Id::new(("mon", label)), resp.hovered(), ANIM_HOVER);
    let sel = anim01(ui.ctx(), Id::new(("monsel", label)), selected, ANIM_VALUE);
    let lift = HOVER_LIFT * hot * (1.0 - sel);
    let r = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top() + 1.0 - lift),
        Pos2::new(rect.right(), rect.bottom() - 1.0 - lift),
    );
    let p = ui.painter();
    p.rect_filled(r, Rounding::same(11.0), fade(pal.control, 0.85));
    if sel > 0.01 {
        p.rect_filled(r, Rounding::same(11.0), fade(mix(pal.accent2, pal.accent, 0.8), 0.20 * sel));
        p.rect_stroke(r, Rounding::same(11.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.28 * sel)));
    } else if hot > 0.01 {
        p.rect_filled(r, Rounding::same(11.0), fade(pal.control, hot));
    }
    // 单选圆
    let c = Pos2::new(r.left() + 18.0, r.center().y);
    p.circle_stroke(c, 6.6, Stroke::new(1.4_f32, mix(pal.dim, pal.accent_bright, sel.max(hot * 0.5))));
    if sel > 0.01 {
        p.circle_filled(c, 3.4 * sel, fade(pal.accent_bright, sel));
    }
    p.text(
        Pos2::new(r.left() + 34.0, r.center().y),
        Align2::LEFT_CENTER,
        ellipsize(label, 40),
        f_sans(12.0),
        mix(pal.label, pal.text, sel.max(hot)),
    );
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
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    let hot = anim_bool(ui.ctx(), Id::new(("kbox", slot)), resp.hovered(), ANIM_HOVER);
    let rec = anim_bool(ui.ctx(), Id::new(("krec", slot)), recording, ANIM_VALUE);
    let mut shapes = Vec::new();
    push_well(&mut shapes, rect, 9.0, pal);
    // 录制中：危险色描边（平滑淡入）+ 外圈光晕；否则 hover 泛紫
    if rec > 0.01 {
        shapes.push(Shape::rect_stroke(rect, Rounding::same(9.0), Stroke::new(1.2_f32, fade(pal.danger, 0.8 * rec))));
        shapes.push(Shape::rect_filled(rect.expand(3.0), Rounding::same(12.0), fade(pal.danger, 0.06 * rec)));
    } else {
        shapes.push(Shape::rect_stroke(
            rect,
            Rounding::same(9.0),
            Stroke::new(1.0_f32, mix(pal.border, pal.accent_bright, hot * 0.75)),
        ));
        if hot > 0.01 {
            shapes.push(Shape::rect_filled(rect.expand(3.0), Rounding::same(12.0), fade(pal.accent, 0.05 * hot)));
        }
    }
    ui.painter().add(Shape::Vec(shapes));
    if recording {
        // 录制脉冲必须每帧重绘（唯一的常驻动画，且只在录制时）
        let t = ui.input(|i| i.time) as f32;
        let pulse = 0.45 + 0.55 * (t * 4.0).sin().abs();
        let c = Pos2::new(rect.left() + 15.0, rect.center().y);
        ui.painter().circle_filled(c, 5.4, fade(pal.danger, 0.22 * pulse));
        ui.painter().circle_filled(c, 3.2, fade(pal.danger, 0.55 + 0.45 * pulse));
        ui.painter().text(
            Pos2::new(c.x + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            f_sans(11.5),
            pal.warn,
        );
        ui.ctx().request_repaint();
    } else {
        ui.painter().text(
            Pos2::new(rect.left() + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            f_sans(12.0),
            if hint { fade(pal.dim, 0.95) } else { mix(pal.text, pal.accent_bright, 0.2) },
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
        field_label(ui, pal, label);
        let (r, g, b) = crate::overlay::parse_hex(target);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::hover());
        let mut shapes = Vec::new();
        push_well(&mut shapes, rect, 8.0, pal);
        ui.painter().add(Shape::Vec(shapes));
        let inner = rect.shrink(3.0);
        ui.painter().rect_filled(inner, Rounding::same(6.0), Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
        ui.painter().rect_stroke(inner, Rounding::same(6.0), Stroke::new(1.0_f32, fade(pal.glass_hi, 0.6)));
        let w = (ui.available_width() - 4.0).clamp(56.0, 150.0);
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
    let hot = anim_bool(ui.ctx(), Id::new(("win", seed)), resp.hovered(), ANIM_HOVER);
    let danger = matches!(kind, WinBtn::Close);
    let r = rect.translate(Vec2::new(0.0, -1.2 * hot));
    let bg = if danger { mix(pal.control, pal.danger, hot) } else { mix(pal.control, pal.hover, hot) };
    if hot > 0.01 {
        ui.painter().rect_filled(r, Rounding::same(9.0), fade(bg, hot));
        ui.painter().rect_stroke(
            r,
            Rounding::same(9.0),
            Stroke::new(1.0_f32, fade(if danger { pal.danger } else { pal.accent_bright }, 0.30 * hot)),
        );
    }
    let fg = if danger && hot > 0.5 { Color32::WHITE } else { mix(pal.label, pal.text, hot) };
    let c = r.center();
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
        // 首帧起每帧调用：窗口就绪后**一次性**加系统圆角 + 亚克力。
        // 返回 `Some(kind)` = 这一帧刚完成尝试（记一条日志，之后恒为 `None`）；
        // 成功与否另有 `blur_active()` 每帧可读，界面据此在高透 / 兜底两组底色之间切。
        if let Some(kind) = crate::system::windowfx::apply_once() {
            self.blur_pending = false;
            info!("设置窗口毛玻璃状态: {kind:?}");
            self.blur_kind = match kind {
                crate::system::windowfx::BlurKind::None => None,
                k => Some(k),
            };
        }
        self.blur_active = crate::system::windowfx::blur_active();
        let dark = match self.theme.as_str() {
            "light" => false,
            "dark" => true,
            // 「自动」跟随系统（eframe 默认把系统主题写进 egui visuals）
            _ => ctx.style().visuals.dark_mode,
        };
        // 本帧时间：卡片入场进度的基准（只播一次，不每帧重放）
        self.enter_now = ctx.input(|i| i.time);
        // 2 秒还没拿到结果（极端环境下枚举不到本进程窗口）→ 不再显示「检测中…」
        if self.blur_pending && self.enter_now > 2.0 {
            self.blur_pending = false;
        }
        // 鼠标平滑跟随：镜面高光与视差共用同一条插值曲线。
        // 按**时间**插值（不是按帧），帧率波动时速度一致；收敛后吸附并停止请求重绘。
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let dt = (self.enter_now - self.fx_t).clamp(0.0, 0.10) as f32;
        self.fx_t = self.enter_now;
        if let Some(target) = hover.or(self.mouse_smooth) {
            let cur = self.mouse_smooth.unwrap_or(target);
            if (target - cur).length() <= MOUSE_SETTLE {
                self.mouse_smooth = Some(target);
            } else {
                let k = 1.0 - (-dt / MOUSE_TAU).exp();
                self.mouse_smooth = Some(cur + (target - cur) * k);
                ctx.request_repaint();
            }
        }
        let mouse = self.mouse_smooth.unwrap_or_else(|| ctx.screen_rect().center());
        // 双模式底色：系统模糊生效 → 真半透明（桌面透得进来，透明度由玻璃强度决定）；
        // 不生效 → 不透明兜底，此时**忽略强度**（`translucent` 整个不参与）
        let mut pal = if dark { Palette::dark() } else { Palette::light() };
        if self.blur_active {
            pal = pal.translucent(self.glass_level);
        }
        pal.fx = Fx { mouse, blur: self.blur_active };
        self.pal = pal;
        let pal = self.pal;
        self.poll_hotkey_recording(ctx);

        // ---- 背板预烘焙：尺寸 / 主题 / 透底 / DPI 任一变化时重建（拖拽缩放期间去抖）----
        let ppp = ctx.pixels_per_point();
        let screen = ctx.screen_rect();
        let size = [
            ((screen.width() * ppp).round() as usize).max(1),
            ((screen.height() * ppp).round() as usize).max(1),
        ];
        // 缓存键必须含**玻璃强度**：强度决定真透底每一层的 alpha，不进键的话调滑杆不会重绘。
        // 无系统模糊时强度被忽略（底色不透明），末位固定 0，避免无效重烘焙。
        let level_q = if pal.fx.blur { (self.glass_level * 100.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
        let key = (size[0], size[1], pal.dark, pal.fx.blur, level_q);
        // 主题 / 透底模式切换**立即**重建；另外把"占位贴图"（首帧尺寸还没定下来、
        // 只有几像素的那种）也当紧急情况——否则它会陪跑整个去抖窗口，看起来就是一块纯色。
        // 连续变化（拖拽缩放、拖玻璃强度滑杆）走去抖（期间沿用旧贴图，几乎无感）。
        let mode_changed = (self.backdrop_key.2, self.backdrop_key.3) != (pal.dark, pal.fx.blur);
        let placeholder = self.backdrop_key.0 < 16 || self.backdrop_key.1 < 16;
        if self.backdrop_key != key {
            let waited = self.backdrop_baked_at.elapsed();
            if self.backdrop.is_none() || mode_changed || placeholder || waited >= BAKE_DEBOUNCE {
                let img = bake_backdrop(size, &pal, ppp);
                match self.backdrop.as_mut() {
                    Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                    None => self.backdrop = Some(ctx.load_texture("acaja-backdrop", img, egui::TextureOptions::LINEAR)),
                }
                self.backdrop_key = key;
                self.backdrop_baked_at = Instant::now();
            } else {
                // 去抖窗口内：预约一次到期重绘——松手后最后一次改动**一定**会落到贴图上，
                // 不会因为不再有输入而停在旧贴图。
                ctx.request_repaint_after(BAKE_DEBOUNCE.saturating_sub(waited));
            }
        }

        egui::CentralPanel::default()
            .frame(Frame::none())
            .show(ctx, |ui| {
                apply_style(ui, &pal);
                let full = ui.max_rect();
                paint_backdrop(ui.painter(), full, &pal, self.backdrop.as_ref().map(|t| t.id()));

                let title_h = 58.0;
                let bottom_h = 62.0;
                let title_rect = Rect::from_min_size(Pos2::new(full.left() + PAD, full.top() + 12.0), Vec2::new(full.width() - PAD * 2.0, title_h));
                self.title_bar(ui, &pal, title_rect);
                let bottom_rect = Rect::from_min_size(
                    Pos2::new(full.left() + PAD, full.bottom() - bottom_h - 16.0),
                    Vec2::new(full.width() - PAD * 2.0, bottom_h),
                );

                let body_top = title_rect.bottom() + 16.0;
                let body_bottom = bottom_rect.top() - 16.0;
                let body_h = (body_bottom - body_top).max(120.0);
                let nav_rect = Rect::from_min_size(Pos2::new(full.left() + PAD, body_top), Vec2::new(NAV_W, body_h));
                let content_rect = Rect::from_min_size(
                    Pos2::new(nav_rect.right() + NAV_GAP, body_top),
                    Vec2::new(full.right() - PAD - NAV_GAP - nav_rect.right(), body_h),
                );

                // ---- 导航（玻璃轨） ----
                let mut nav_shapes = Vec::new();
                push_panel(&mut nav_shapes, nav_rect, R_CARD, &pal, 0.0);
                ui.painter().add(Shape::Vec(nav_shapes));
                let mut nav_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(nav_rect.shrink2(Vec2::new(10.0, 14.0)))
                        .layout(Layout::top_down(Align::Min)),
                );
                nav_ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
                self.nav_ui(&mut nav_ui, &pal);

                // ---- 区块序号水印（衬线大字，极低透明度）：大留白里的视觉锚点 ----
                let wm = format!("{:02}", self.active_section + 1);
                ui.painter().text(
                    Pos2::new(content_rect.right() - 4.0, content_rect.top() - 34.0),
                    Align2::RIGHT_TOP,
                    wm,
                    f_serif(150.0),
                    fade(pal.text, if pal.dark { 0.055 } else { 0.07 }),
                );

                // ---- 内容（滚动；左右各留 14px、顶部 16px 给外阴影与入场上移） ----
                let mut content_ui = ui.new_child(
                    UiBuilder::new().max_rect(content_rect).layout(Layout::top_down(Align::Min)),
                );
                content_ui.set_clip_rect(content_rect);
                content_ui.spacing_mut().item_spacing = Vec2::new(0.0, GROUP_GAP);
                ScrollArea::vertical().auto_shrink([false, false]).show(&mut content_ui, |ui| {
                    apply_style(ui, &pal);
                    let ar = ui.available_rect_before_wrap();
                    let inner = Rect::from_min_max(
                        Pos2::new(ar.left() + 14.0, ar.top() + 16.0),
                        Pos2::new(ar.right() - 14.0, ar.bottom()),
                    );
                    let mut sui = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
                    sui.spacing_mut().item_spacing = Vec2::new(0.0, GROUP_GAP);
                    match self.active_section {
                        SEC_STYLE => self.section_style(&mut sui, &pal),
                        SEC_DYNAMIC => self.section_dynamic(&mut sui, &pal),
                        SEC_POSITION => self.section_position(&mut sui, &pal),
                        SEC_GAMEPAD => self.section_gamepad(&mut sui, &pal),
                        SEC_HOTKEY => self.section_hotkey(&mut sui, &pal),
                        SEC_IMAGE => self.section_image(&mut sui, &pal),
                        SEC_PRESETS => self.section_presets(&mut sui, &pal),
                        _ => self.section_system(&mut sui, &pal),
                    }
                    sui.add_space(28.0);
                    // 把内缩子 Ui 的实际高度交回给滚动区，否则内容会被当成 0 高
                    ui.advance_cursor_after_rect(sui.min_rect());
                });

                // ---- 底部操作条 ----
                self.bottom_bar(ui, &pal, bottom_rect);

                // ---- 无边框窗口的自绘缩放边 ----
                resize_grips(ui, full);
            });

        // ---- 入场动画进行中才请求重绘（静止时完全不动，省电） ----
        // 覆盖到最后一个错峰序号（当前最多 4 张卡）
        if self.enter_now - self.enter_t0 < (CARD_ENTER_DUR + ENTER_STAGGER * 4.0) as f64 {
            ctx.request_repaint();
        }

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

