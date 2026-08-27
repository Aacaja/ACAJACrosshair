//! 配置中心：预设 schema、JSON 持久化、旧版 CrossHairLIN 迁移。
//!
//! 目录布局（`%APPDATA%/ACAJACrosshair/`）：
//! ```text
//! app.json            # 应用级设置（语言/主题/自启/游戏绑定）
//! presets/default.json # 预设，每个预设一个文件
//! presets/<name>.json
//! ```
//!
//! 兼容性约定：
//! - 所有预设字段均 `#[serde(default)]`，旧/缺省 JSON 用默认值补齐；
//! - 未知字段忽略（`deny_unknown_fields` 不开），向前兼容；
//! - 旧版 `%APPDATA%/CrosshairApp/*.json` 首次启动自动迁移（见 [`migrate_legacy`]）。

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::Lang;

/// 配置文件 schema 版本
pub const SCHEMA_VERSION: u32 = 1;

// ===========================================================================
// 值类型
// ===========================================================================

/// 准星形状（v1 对齐 Crosshair X 的 14 种；旧版 8 种命名保持不变以便迁移）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    #[default]
    Cross,
    Dot,
    Square,
    Circle,
    HollowCross,
    HollowSquare,
    HollowCrossDot,
    Chevron,
    Triangle,
    VShape,
    TShape,
    Brackets,
    GapHair,
    CustomImage,
}

impl Shape {
    /// 全部形状（UI 下拉框顺序）
    pub const ALL: [Shape; 14] = [
        Shape::Cross,
        Shape::Dot,
        Shape::Square,
        Shape::Circle,
        Shape::HollowCross,
        Shape::HollowSquare,
        Shape::HollowCrossDot,
        Shape::Chevron,
        Shape::Triangle,
        Shape::VShape,
        Shape::TShape,
        Shape::Brackets,
        Shape::GapHair,
        Shape::CustomImage,
    ];

    pub fn is_custom_image(self) -> bool {
        self == Shape::CustomImage
    }
}

/// 位置上值：居中或像素坐标（沿用旧版 JSON 的 `"center"` / 数字 表示）
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum PosVal {
    #[default]
    Center,
    Px(f32),
}

impl Serialize for PosVal {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            PosVal::Center => s.serialize_str("center"),
            PosVal::Px(v) => s.serialize_f32(*v),
        }
    }
}

impl<'de> Deserialize<'de> for PosVal {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Str(String),
            Num(f32),
        }
        match Raw::deserialize(d)? {
            Raw::Str(s) if s.eq_ignore_ascii_case("center") => Ok(PosVal::Center),
            Raw::Str(_) => Err(serde::de::Error::custom("invalid position value")),
            Raw::Num(v) => Ok(PosVal::Px(v)),
        }
    }
}

/// 准星位置 + 显示器编号
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Position {
    pub x: PosVal,
    pub y: PosVal,
    /// 目标显示器索引（-1 = 跟随主屏/前台窗口所在屏）
    #[serde(default = "default_monitor")]
    pub monitor: i32,
}

fn default_monitor() -> i32 {
    -1
}

impl Default for Position {
    fn default() -> Self {
        Position { x: PosVal::Center, y: PosVal::Center, monitor: -1 }
    }
}

/// 快捷键修饰键位（位标志）
pub const MOD_NONE: u32 = 0;
pub const MOD_ALT: u32 = 1;
pub const MOD_CONTROL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

/// 全局快捷键：序列化为 "Ctrl+Alt+F8" 风格字符串；兼容解析旧版 Qt 字符串
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Hotkey {
    pub modifiers: u32,
    /// Windows 虚拟键码 (VK_*)
    pub vk: u32,
}

impl Hotkey {
    pub fn is_empty(&self) -> bool {
        self.vk == 0
    }

    /// 解析 "Ctrl+Shift+F1" 风格字符串（兼容旧版 Qt QKeySequence toString 输出）
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut modifiers = MOD_NONE;
        let mut vk: u32 = 0;
        for token in s.split('+').map(|t| t.trim()).filter(|t| !t.is_empty()) {
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers |= MOD_CONTROL,
                "alt" => modifiers |= MOD_ALT,
                "shift" => modifiers |= MOD_SHIFT,
                "win" | "meta" | "cmd" => modifiers |= MOD_WIN,
                _ => vk = key_token_to_vk(token)?,
            }
        }
        if vk == 0 {
            return None;
        }
        Some(Hotkey { modifiers, vk })
    }
}

/// 将单个按键名（"F1"、"A"、"Space"…）映射为 Windows 虚拟键码
fn key_token_to_vk(token: &str) -> Option<u32> {
    let upper = token.to_ascii_uppercase();
    match upper.as_str() {
        // F1-F24
        t if t.len() == 2 && t.starts_with('F') => {
            let n: u32 = t[1..].parse().ok()?;
            if (1..=24).contains(&n) {
                Some(0x70 + n - 1)
            } else {
                None
            }
        }
        // A-Z
        t if t.len() == 1 && t.as_bytes()[0].is_ascii_uppercase() => {
            Some(0x41 + u32::from(t.as_bytes()[0] - b'A'))
        }
        // 0-9
        t if t.len() == 1 && t.as_bytes()[0].is_ascii_digit() => {
            Some(0x30 + u32::from(t.as_bytes()[0] - b'0'))
        }
        "SPACE" => Some(0x20),
        "TAB" => Some(0x09),
        "ENTER" | "RETURN" => Some(0x0D),
        "ESC" | "ESCAPE" => Some(0x1B),
        "BACKSPACE" => Some(0x08),
        "DELETE" | "DEL" => Some(0x2E),
        "INSERT" | "INS" => Some(0x2D),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PGUP" | "PAGEUP" => Some(0x21),
        "PGDN" | "PAGEDOWN" => Some(0x22),
        "UP" => Some(0x26),
        "DOWN" => Some(0x28),
        "LEFT" => Some(0x25),
        "RIGHT" => Some(0x27),
        "-" | "MINUS" => Some(0xBD),
        "=" | "EQUALS" => Some(0xBB),
        "[" => Some(0xDB),
        "]" => Some(0xDD),
        "\\" => Some(0xDC),
        ";" | "SEMICOLON" => Some(0xBA),
        "'" | "QUOTE" => Some(0xDE),
        "," | "COMMA" => Some(0xBC),
        "." | "PERIOD" => Some(0xBE),
        "/" | "SLASH" => Some(0xBF),
        "`" | "GRAVE" => Some(0xC0),
        _ => None,
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.vk == 0 {
            return write!(f, "");
        }
        let mut parts = Vec::new();
        if self.modifiers & MOD_CONTROL != 0 {
            parts.push("Ctrl");
        }
        if self.modifiers & MOD_ALT != 0 {
            parts.push("Alt");
        }
        if self.modifiers & MOD_SHIFT != 0 {
            parts.push("Shift");
        }
        if self.modifiers & MOD_WIN != 0 {
            parts.push("Win");
        }
        parts.push(vk_to_key_token(self.vk));
        write!(f, "{}", parts.join("+"))
    }
}

fn vk_to_key_token(vk: u32) -> &'static str {
    match vk {
        0x70..=0x87 => {
            // F1-F24
            static BUF: [&str; 24] = [
                "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13",
                "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
            ];
            BUF[(vk - 0x70) as usize]
        }
        0x41..=0x5A => {
            static BUF: [&str; 26] = [
                "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
                "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
            ];
            BUF[(vk - 0x41) as usize]
        }
        0x30..=0x39 => {
            static BUF: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
            BUF[(vk - 0x30) as usize]
        }
        0x20 => "Space",
        0x09 => "Tab",
        0x0D => "Enter",
        0x1B => "Esc",
        0x08 => "Backspace",
        0x2E => "Delete",
        0x2D => "Insert",
        0x24 => "Home",
        0x23 => "End",
        0x21 => "PgUp",
        0x22 => "PgDn",
        0x26 => "Up",
        0x28 => "Down",
        0x25 => "Left",
        0x27 => "Right",
        _ => "?",
    }
}

impl Serialize for Hotkey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Hotkey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s.is_empty() {
            return Ok(Hotkey::default());
        }
        Hotkey::parse(&s).ok_or_else(|| serde::de::Error::custom("invalid hotkey string"))
    }
}

/// 手柄 ADS（瞄准）模式 —— Apex 手柄需求的核心配置
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdsMode {
    /// 不介入
    Off,
    /// 按住瞄准键 → 隐藏准星，松开恢复（默认）
    #[default]
    HoldHide,
    /// 每次扣满扳机切换一次显隐
    Toggle,
    /// 反向：按住显示、松开隐藏
    HoldShow,
}

/// 瞄准触发键
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AdsButton {
    /// 左扳机 LT / L2（Apex 默认瞄准键）
    #[default]
    LeftTrigger,
    RightTrigger,
}

/// 手柄相关配置（每个预设独立保存，可随游戏/图内模式切换）
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct GamepadConfig {
    pub ads_mode: AdsMode,
    pub ads_button: AdsButton,
    /// 触发阈值 0-255（XInput 模拟量），默认 30
    #[serde(default = "default_trigger_threshold")]
    pub trigger_threshold: u8,
    /// 使用右扳机开火驱动动态准星扩散
    #[serde(default)]
    pub fire_expand: bool,
}

fn default_trigger_threshold() -> u8 {
    30
}

impl Default for GamepadConfig {
    fn default() -> Self {
        GamepadConfig {
            ads_mode: AdsMode::HoldHide,
            ads_button: AdsButton::LeftTrigger,
            trigger_threshold: default_trigger_threshold(),
            fire_expand: false,
        }
    }
}

/// 动态准星（开火扩散 + 后坐力恢复指示）
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct DynamicConfig {
    /// 开火时扩散（像素）
    #[serde(default)]
    pub fire_expand_px: f32,
    /// 扩散恢复速度（毫秒/像素收回）
    #[serde(default = "default_recover_ms")]
    pub recover_ms: u32,
    /// 后坐力恢复指示条（示意当前扩散程度）
    #[serde(default)]
    pub recoil_indicator: bool,
}

fn default_recover_ms() -> u32 {
    120
}

impl Default for DynamicConfig {
    fn default() -> Self {
        DynamicConfig { fire_expand_px: 0.0, recover_ms: default_recover_ms(), recoil_indicator: false }
    }
}

/// 描边 / 轮廓
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct OutlineConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_outline_color")]
    pub color: String,
    #[serde(default = "default_outline_opacity")]
    pub opacity: f32,
    #[serde(default = "default_outline_thickness")]
    pub thickness: f32,
}

fn default_outline_color() -> String {
    "#000000".to_string()
}
fn default_outline_opacity() -> f32 {
    1.0
}
fn default_outline_thickness() -> f32 {
    1.0
}

impl Default for OutlineConfig {
    fn default() -> Self {
        OutlineConfig {
            enabled: false,
            color: default_outline_color(),
            opacity: default_outline_opacity(),
            thickness: default_outline_thickness(),
        }
    }
}

/// 空心十字参数
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct HollowConfig {
    /// 中心缺口
    #[serde(default)]
    pub gap: f32,
    /// 空心十字中心点大小
    #[serde(default)]
    pub center_dot_size: f32,
}

impl Default for HollowConfig {
    fn default() -> Self {
        HollowConfig { gap: 0.0, center_dot_size: 3.0 }
    }
}

/// 自定义图片
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
pub struct ImageConfig {
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_image_scale")]
    pub scale: f32,
}

fn default_image_scale() -> f32 {
    1.0
}

/// 四象限独立颜色（对齐 Crosshair X 多色准星）。
/// 全部相等时等同于单色准星。
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct QuadColors {
    pub top: String,
    pub bottom: String,
    pub left: String,
    pub right: String,
}

impl Default for QuadColors {
    fn default() -> Self {
        QuadColors {
            top: "#FF0000".into(),
            bottom: "#FF0000".into(),
            left: "#FF0000".into(),
            right: "#FF0000".into(),
        }
    }
}

impl QuadColors {
    pub fn from_single(color: &str) -> Self {
        QuadColors {
            top: color.into(),
            bottom: color.into(),
            left: color.into(),
            right: color.into(),
        }
    }
    pub fn is_uniform(&self) -> bool {
        self.top == self.bottom && self.top == self.left && self.top == self.right
    }
}

// ===========================================================================
// 预设
// ===========================================================================

/// 单个预设（对应旧版一个 .json 配置文件）
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Preset {
    #[serde(default = "default_schema")]
    pub schema: u32,

    // -- 外观 --
    #[serde(default)]
    pub shape: Shape,
    #[serde(default = "default_size")]
    pub size: f32,
    #[serde(default = "default_thickness")]
    pub thickness: f32,
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// 主色（十六进制 #RRGGBB）
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub colors: QuadColors,
    /// 多色模式开关（false = 统一使用 color）
    #[serde(default)]
    pub multicolor: bool,
    /// 旋转角度（度）
    #[serde(default)]
    pub rotation: f32,
    #[serde(default)]
    pub outline: OutlineConfig,
    #[serde(default)]
    pub hollow: HollowConfig,
    #[serde(default)]
    pub image: ImageConfig,
    #[serde(default)]
    pub dynamic: DynamicConfig,

    // -- 位置 --
    #[serde(default)]
    pub position: Position,
    /// 窗口吸附模式（跟随前台游戏窗口）
    #[serde(default)]
    pub snap_to_window: bool,

    // -- 输入 --
    #[serde(default)]
    pub hotkey_toggle: Hotkey,
    #[serde(default)]
    pub hotkey_next_profile: Hotkey,
    #[serde(default)]
    pub right_click_toggle: bool,
    /// 右键模式：click / hold_show / hold_hide
    #[serde(default = "default_right_click_mode")]
    pub right_click_mode: RightClickMode,
    #[serde(default)]
    pub gamepad: GamepadConfig,

    // -- 系统 --
    #[serde(default)]
    pub auto_topmost: bool,
}

fn default_schema() -> u32 {
    SCHEMA_VERSION
}
fn default_size() -> f32 {
    20.0
}
fn default_thickness() -> f32 {
    2.0
}
fn default_opacity() -> f32 {
    0.8
}
fn default_color() -> String {
    "#FF0000".to_string()
}

/// 右键切换模式
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightClickMode {
    Click,
    HoldShow,
    HoldHide,
}

fn default_right_click_mode() -> RightClickMode {
    RightClickMode::Click
}

impl Default for Preset {
    fn default() -> Self {
        Preset {
            schema: SCHEMA_VERSION,
            shape: Shape::Cross,
            size: default_size(),
            thickness: default_thickness(),
            opacity: default_opacity(),
            color: default_color(),
            colors: QuadColors::default(),
            multicolor: false,
            rotation: 0.0,
            outline: OutlineConfig::default(),
            hollow: HollowConfig::default(),
            image: ImageConfig::default(),
            dynamic: DynamicConfig::default(),
            position: Position::default(),
            snap_to_window: false,
            hotkey_toggle: Hotkey::default(),
            hotkey_next_profile: Hotkey::default(),
            right_click_toggle: false,
            right_click_mode: RightClickMode::Click,
            gamepad: GamepadConfig::default(),
            auto_topmost: false,
        }
    }
}

impl Preset {
    /// 当前生效的颜色（多色模式取四象限，否则主色）
    pub fn active_colors(&self) -> QuadColors {
        if self.multicolor {
            self.colors.clone()
        } else {
            QuadColors::from_single(&self.color)
        }
    }
}

// ===========================================================================
// 应用级配置
// ===========================================================================

/// 游戏绑定：前台进程 exe 名 → 预设名（Crosshair X 的 per-game profile）
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct GameBinding {
    /// 进程文件名（不含路径，大小写不敏感，如 "r5apex.exe"）
    pub exe: String,
    /// 绑定的预设名
    pub preset: String,
}

/// 应用级配置 app.json
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_schema")]
    pub schema: u32,
    #[serde(default)]
    pub language: String,
    /// auto / light / dark
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub last_preset: String,
    #[serde(default)]
    pub game_bindings: Vec<GameBinding>,
}

fn default_theme() -> String {
    "auto".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            schema: SCHEMA_VERSION,
            language: "zh".to_string(),
            theme: default_theme(),
            minimize_to_tray: false,
            autostart: false,
            last_preset: "default".to_string(),
            game_bindings: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn lang(&self) -> Lang {
        Lang::from_code(&self.language)
    }
}

// ===========================================================================
// 持久化
// ===========================================================================

/// 预设仓库：管理预设文件的增删改查（原子写：临时文件 + rename）
#[derive(Debug)]
pub struct PresetStore {
    dir: PathBuf,
    pub app: AppConfig,
    presets: BTreeMap<String, Preset>,
}

const APP_FILE: &str = "app.json";
const PRESETS_DIR: &str = "presets";

impl PresetStore {
    /// 打开/创建配置仓库（目录不存在时创建）
    pub fn open(dir: &Path) -> io::Result<PresetStore> {
        fs::create_dir_all(dir)?;
        fs::create_dir_all(dir.join(PRESETS_DIR))?;

        let app = Self::load_app(&dir.join(APP_FILE));
        let mut store = PresetStore { dir: dir.to_path_buf(), app, presets: BTreeMap::new() };

        // 加载全部预设文件
        for entry in fs::read_dir(store.dir.join(PRESETS_DIR))? {
            let entry = entry?;
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(preset) = Self::load_preset_file(&entry.path()) {
                let name = entry
                    .path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                store.presets.insert(name, preset);
            }
        }
        // 确保默认预设存在
        if !store.presets.contains_key("default") {
            store.presets.insert("default".to_string(), Preset::default());
        }
        Ok(store)
    }

    fn load_app(path: &Path) -> AppConfig {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("app.json 解析失败({e})，使用默认配置");
                AppConfig::default()
            }),
            Err(_) => AppConfig::default(),
        }
    }

    fn load_preset_file(path: &Path) -> serde_json::Result<Preset> {
        let text = fs::read_to_string(path)
            .map_err(|e| serde_json::Error::io(e))?;
        serde_json::from_str(&text)
    }

    /// 原子写 JSON（先写 .tmp 再 rename，避免写一半损坏）
    fn write_json_atomic(path: &Path, value: &impl Serialize) -> io::Result<()> {        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(value)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&tmp, text)?;
        match fs::rename(&tmp, path) {
            Ok(()) => Ok(()),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    // ---- 查询 ----

    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn preset_names(&self) -> Vec<String> {
        self.presets.keys().cloned().collect()
    }
    pub fn get(&self, name: &str) -> Option<&Preset> {
        self.presets.get(name)
    }
    pub fn get_active(&self) -> &Preset {
        let name = &self.app.last_preset;
        self.presets.get(name).unwrap_or_else(|| self.presets.get("default").unwrap())
    }
    pub fn active_name(&self) -> String {
        let name = &self.app.last_preset;
        if self.presets.contains_key(name) {
            name.clone()
        } else {
            "default".to_string()
        }
    }

    // ---- 写操作 ----

    pub fn save_preset(&mut self, name: &str, preset: &Preset) -> io::Result<()> {
        let path = self.dir.join(PRESETS_DIR).join(format!("{name}.json"));
        Self::write_json_atomic(&path, preset)?;
        self.presets.insert(name.to_string(), preset.clone());
        Ok(())
    }

    pub fn delete_preset(&mut self, name: &str) -> io::Result<bool> {
        if name == "default" {
            return Ok(false);
        }
        let path = self.dir.join(PRESETS_DIR).join(format!("{name}.json"));
        if path.exists() {
            fs::remove_file(&path)?;
        }
        self.presets.remove(name);
        if self.app.last_preset == name {
            self.app.last_preset = "default".to_string();
            let _ = self.save_app();
        }
        Ok(true)
    }

    /// 激活预设并持久化 last_preset
    pub fn activate(&mut self, name: &str) -> bool {
        if self.presets.contains_key(name) {
            self.app.last_preset = name.to_string();
            let _ = self.save_app();
            true
        } else {
            false
        }
    }

    pub fn save_app(&self) -> io::Result<()> {
        Self::write_json_atomic(&self.dir.join(APP_FILE), &self.app)
    }
}

// ===========================================================================
// 旧版 CrossHairLIN 迁移（%APPDATA%/CrosshairApp/*.json）
// ===========================================================================

/// 迁移结果汇总
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MigrationReport {
    /// 已迁移的预设数
    pub presets: usize,
    /// 失败的文件
    pub failed: Vec<String>,
}

/// 旧版 app_state.json（部分字段）
#[derive(Deserialize, Default)]
struct LegacyState {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    last_preset: Option<String>,
}

/// 尝试从旧版目录迁移配置。仅在目标目录没有任何 JSON 时执行；旧目录保留不删。
///
/// 若旧版语言设置存在则写入新 app.json。
pub fn migrate_legacy(old_dir: &Path, new_dir: &Path) -> io::Result<MigrationReport> {
    let mut report = MigrationReport::default();

    // 仅当新配置目录尚未初始化（无 app.json 且无预设）时迁移
    let new_has_data = new_dir.join(APP_FILE).exists()
        || fs::read_dir(new_dir.join(PRESETS_DIR)).map(|mut it| it.next().is_some()).unwrap_or(false);
    if new_has_data || !old_dir.exists() {
        return Ok(report);
    }

    fs::create_dir_all(new_dir.join(PRESETS_DIR))?;

    // 应用级：语言与上次预设
    let state_path = old_dir.join("app_state.json");
    if state_path.exists() {
        if let Ok(text) = fs::read_to_string(&state_path) {
            if let Ok(state) = serde_json::from_str::<LegacyState>(&text) {
                let mut app = AppConfig::default();
                if let Some(lang) = state.language {
                    app.language = lang;
                }
                app.last_preset = state.last_preset.unwrap_or_else(|| "default".into());
                let _ = PresetStore::write_json_atomic(&new_dir.join(APP_FILE), &app);
            }
        }
    }

    // 逐个迁移预设文件（跳过 app_state.json）
    for entry in fs::read_dir(old_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some("app_state") {
            continue;
        }
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("default").to_string();
        match migrate_legacy_preset(&path) {
            Ok(preset) => {
                let target = new_dir.join(PRESETS_DIR).join(format!("{name}.json"));
                if PresetStore::write_json_atomic(&target, &preset).is_ok() {
                    report.presets += 1;
                } else {
                    report.failed.push(name);
                }
            }
            Err(_) => report.failed.push(name),
        }
    }
    Ok(report)
}

/// 旧版单预设 JSON → 新 Preset
fn migrate_legacy_preset(path: &Path) -> serde_json::Result<Preset> {
    let text = fs::read_to_string(path).map_err(serde_json::Error::io)?;
    let v: serde_json::Value = serde_json::from_str(&text)?;
    Ok(convert_legacy_value(&v))
}

/// 映射旧版字段 → 新 schema（缺省字段用新默认值）
fn convert_legacy_value(v: &serde_json::Value) -> Preset {
    let mut p = Preset::default();

    let get = |k: &str| v.get(k).and_then(|x| x.as_str());
    let get_f = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
    let get_b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);

    // 形状（旧版命名与新版一致）
    if let Some(s) = get("shape") {
        p.shape = serde_json::from_str(&format!("\"{s}\"")).unwrap_or(Shape::Cross);
    }
    p.size = get_f("size").unwrap_or(p.size);
    p.thickness = get_f("thickness").unwrap_or(p.thickness);
    p.opacity = get_f("opacity").unwrap_or(p.opacity);
    if let Some(c) = get("color") {
        p.color = c.to_string();
    }

    // 位置：旧版 {x: "center"|int, y: ...}
    if let Some(pos) = v.get("position").and_then(|x| x.as_object()) {
        let parse = |k: &str| -> PosVal {
            match pos.get(k) {
                Some(serde_json::Value::String(s)) if s == "center" => PosVal::Center,
                Some(serde_json::Value::Number(n)) => PosVal::Px(n.as_f64().unwrap_or(0.0) as f32),
                _ => PosVal::Center,
            }
        };
        p.position = Position { x: parse("x"), y: parse("y"), monitor: -1 };
    }

    // 快捷键
    if let Some(hk) = get("hotkey") {
        p.hotkey_toggle = Hotkey::parse(hk).unwrap_or_default();
    }

    // 右键切换
    p.right_click_toggle = get_b("right_click_shortcut");
    if let Some(m) = get("right_click_mode") {
        p.right_click_mode = match m {
            "hold_show" => RightClickMode::HoldShow,
            "hold_hide" => RightClickMode::HoldHide,
            _ => RightClickMode::Click,
        };
    }

    // 描边
    p.outline.enabled = get_b("enable_border");
    p.outline.color = get("border_color").unwrap_or("#000000").to_string();
    p.outline.opacity = get_f("border_opacity").unwrap_or(1.0);
    p.outline.thickness = get_f("border_thickness").unwrap_or(1.0);

    // 空心十字
    p.hollow.gap = get_f("hollow_gap").unwrap_or(0.0);
    p.hollow.center_dot_size = get_f("center_dot_size").unwrap_or(3.0);

    // 自定义图片
    p.image.path = get("custom_image_path").unwrap_or("").to_string();
    p.image.scale = get_f("custom_image_scale").unwrap_or(1.0);

    // 系统
    p.auto_topmost = get_b("auto_topmost_on_fullscreen");

    p
}

// ===========================================================================
// 测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("acaja_test_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn preset_default_roundtrip() {
        let p = Preset::default();
        let json = serde_json::to_string_pretty(&p).unwrap();
        let back: Preset = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn hotkey_parse_and_display() {
        let hk = Hotkey::parse("Ctrl+Shift+F1").unwrap();
        assert_eq!(hk.modifiers, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(hk.vk, 0x70);
        assert_eq!(hk.to_string(), "Ctrl+Shift+F1");

        let hk2 = Hotkey::parse("Alt+F8").unwrap();
        assert_eq!(hk2.to_string(), "Alt+F8");

        // 旧版 Qt 字符串兼容
        let hk3 = Hotkey::parse("Ctrl+F1").unwrap();
        assert_eq!(hk3.to_string(), "Ctrl+F1");

        // 非法
        assert!(Hotkey::parse("").is_none());
        assert!(Hotkey::parse("Ctrl+").is_none());
        assert!(Hotkey::parse("ZZZ").is_none());
    }

    #[test]
    fn position_center_roundtrip() {
        let pos = Position::default();
        let json = serde_json::to_string(&pos).unwrap();
        assert!(json.contains("\"center\""));
        let back: Position = serde_json::from_str(&json).unwrap();
        assert_eq!(back.x, PosVal::Center);

        let pos2 = Position { x: PosVal::Px(960.0), y: PosVal::Px(540.0), monitor: 1 };
        let json2 = serde_json::to_string(&pos2).unwrap();
        let back2: Position = serde_json::from_str(&json2).unwrap();
        assert_eq!(back2.x, PosVal::Px(960.0));
        assert_eq!(back2.monitor, 1);
    }

    #[test]
    fn store_save_load_roundtrip() {
        let dir = temp_dir("store");
        let mut store = PresetStore::open(&dir).unwrap();
        assert_eq!(store.preset_names(), vec!["default".to_string()]);

        let mut p = Preset::default();
        p.shape = Shape::Circle;
        p.size = 45.0;
        p.gamepad.ads_mode = AdsMode::Toggle;
        store.save_preset("apex", &p).unwrap();

        let store2 = PresetStore::open(&dir).unwrap();
        let loaded = store2.get("apex").unwrap();
        assert_eq!(loaded.shape, Shape::Circle);
        assert_eq!(loaded.size, 45.0);
        assert_eq!(loaded.gamepad.ads_mode, AdsMode::Toggle);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_preset_guards_default() {
        let dir = temp_dir("delete");
        let mut store = PresetStore::open(&dir).unwrap();
        assert!(!store.delete_preset("default").unwrap());
        store.save_preset("x", &Preset::default()).unwrap();
        assert!(store.delete_preset("x").unwrap());
        assert!(!store.preset_names().contains(&"x".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = temp_dir("corrupt");
        let store = PresetStore::open(&dir).unwrap();
        fs::write(dir.join("presets").join("broken.json"), "{ not json !!!").unwrap();
        let store = PresetStore::open(&dir).unwrap();
        // 损坏文件被忽略，default 仍存在
        assert!(store.get("default").is_some());
        assert!(!store.preset_names().contains(&"broken".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_legacy_presets() {
        // 构造旧版目录
        let old = temp_dir("legacy_old");
        let new = temp_dir("legacy_new");
        let legacy_json = r#"{
            "shape": "hollow_cross_dot",
            "size": 32,
            "thickness": 2,
            "opacity": 0.85,
            "color": "#00FF00",
            "position": {"x": 100, "y": "center"},
            "hotkey": "Ctrl+F2",
            "right_click_shortcut": true,
            "right_click_mode": "hold_show",
            "enable_border": true,
            "border_thickness": 3,
            "border_color": "#123456",
            "border_opacity": 0.5,
            "custom_image_path": "C:/img.png",
            "custom_image_scale": 1.5,
            "hollow_gap": 8,
            "center_dot_size": 5,
            "auto_topmost_on_fullscreen": true,
            "language": "en",
            "opacity_extra": "ignored_unknown_field"
        }"#;
        fs::write(old.join("apex.json"), legacy_json).unwrap();
        fs::write(
            old.join("app_state.json"),
            r#"{"language": "en", "last_preset": "apex"}"#,
        )
        .unwrap();

        let report = migrate_legacy(&old, &new).unwrap();
        assert_eq!(report.presets, 1);
        assert!(report.failed.is_empty());

        // 校验迁移结果
        let store = PresetStore::open(&new).unwrap();
        let apex = store.get("apex").unwrap();
        assert_eq!(apex.shape, Shape::HollowCrossDot);
        assert_eq!(apex.size, 32.0);
        assert_eq!(apex.thickness, 2.0);
        assert_eq!(apex.color, "#00FF00");
        assert_eq!(apex.position.x, PosVal::Px(100.0));
        assert_eq!(apex.position.y, PosVal::Center);
        assert_eq!(apex.hotkey_toggle.to_string(), "Ctrl+F2");
        assert!(apex.right_click_toggle);
        assert_eq!(apex.right_click_mode, RightClickMode::HoldShow);
        assert!(apex.outline.enabled);
        assert_eq!(apex.outline.thickness, 3.0);
        assert_eq!(apex.outline.color, "#123456");
        assert_eq!(apex.outline.opacity, 0.5);
        assert_eq!(apex.image.path, "C:/img.png");
        assert_eq!(apex.image.scale, 1.5);
        assert_eq!(apex.hollow.gap, 8.0);
        assert_eq!(apex.hollow.center_dot_size, 5.0);
        assert!(apex.auto_topmost);

        // 语言已迁移到 app.json
        assert_eq!(store.app.language, "en");
        assert_eq!(store.app.last_preset, "apex");
        let _ = fs::remove_dir_all(&old);
        let _ = fs::remove_dir_all(&new);
    }

    #[test]
    fn migrate_skipped_when_new_has_data() {
        let old = temp_dir("legacy_skip_old");
        let new = temp_dir("legacy_skip_new");
        fs::write(old.join("default.json"), r#"{"size": 10}"#).unwrap();
        // 新目录已有 app.json
        fs::write(new.join("app.json"), "{}").unwrap();

        let report = migrate_legacy(&old, &new).unwrap();
        assert_eq!(report.presets, 0);
        let _ = fs::remove_dir_all(&old);
        let _ = fs::remove_dir_all(&new);
    }

    #[test]
    fn ads_mode_defaults() {
        let g = GamepadConfig::default();
        assert_eq!(g.ads_mode, AdsMode::HoldHide);
        assert_eq!(g.ads_button, AdsButton::LeftTrigger);
        assert_eq!(g.trigger_threshold, 30);
    }
}