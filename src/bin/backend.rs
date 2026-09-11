//! ACAJA 入口（v1.1.0 双进程架构）。
//!
//! - 默认启动 = **后台壳进程**：D2D 准星 + 输入 + 托盘，内存 ~15MB；
//!   设置界面按需以独立进程 `acaja.exe --ui` 拉起，修改参数经 WM_COPYDATA 实时同步。
//! - `--ui` 参数 = **设置 UI 进程**：egui 界面；关闭即释放全部界面资源（~65MB）；
//!   每 2s 自检主进程窗口，主进程退出时自动退出。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use crossbeam_channel::{Receiver, Sender, unbounded};
use log::{info, warn};
use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, PeekMessageW,
    RegisterClassW, TranslateMessage, WNDCLASSW, WNDCLASS_STYLES,
    MSG, PM_REMOVE, WM_CONTEXTMENU, WM_COPYDATA, WM_HOTKEY, WM_INPUT, WM_LBUTTONDBLCLK,
    WM_QUIT, WM_RBUTTONUP, WS_POPUP, WINDOW_EX_STYLE, WINDOW_STYLE,
};
use windows::core::{PCWSTR, w};

use acaja::config::{migrate_legacy, Preset, PresetStore};
use acaja::input::gamepad::{GameEvent, RuntimeGamepadCfg, start_gamepad};
use acaja::input::raw_mouse::{RawMouseEvent, handle_raw_input, register_raw_mouse};
use acaja::ipc::IPC_TAG_PRESET;
use acaja::overlay::OverlayHandle;
use acaja::state::{AppState, SharedPreset, apply_ads_event, next_preset, snap_position};
use acaja::system::foreground::{FgEvent, start_fg_watcher};
use acaja::system::hotkey::{HOTKEY_ID_NEXT_PRESET, HOTKEY_ID_TOGGLE, register as reg_hotkey, unregister as unreg_hotkey};
use acaja::system::tray::{self, CMD_QUIT, CMD_SETTINGS, CMD_TOGGLE, Tray, WM_TRAYICON};
use acaja::ui::strings::t;
use acaja::{APP_NAME, VERSION};

const MAIN_CLASS: PCWSTR = w!("ACAJAMainWindow");

// ===========================================================================
// 主状态（后台壳进程的消息线程）
// ===========================================================================

struct MainState {
    store: Arc<Mutex<PresetStore>>,
    overlay: OverlayHandle,
    hwnd: HWND,
    tray: Option<Tray>,
    app: AppState,
    shared: Arc<RwLock<SharedPreset>>,
    last_version: u64,
    gamepad_cfg: Arc<RwLock<RuntimeGamepadCfg>>,
    fg_exe: String,
    /// 配置文件监控（v1.1.5「推送按钮」模式：UI 修改文件 → 主进程自动应用）
    watched: FileWatch,
}

/// 文件监控状态：app.json 与当前预设文件的修改时间
struct FileWatch {
    app_mtime: Option<std::time::SystemTime>,
    preset_mtime: Option<std::time::SystemTime>,
    preset_name: String,
}

impl MainState {
    fn preset(&self) -> Arc<Preset> {
        self.shared.read().preset.clone()
    }

    fn position_for_preset(&self) -> (i32, i32) {
        let p = self.preset();
        acaja::state::resolve_position(&p)
    }

    fn full_update(&mut self) {
        let pos = self.position_for_preset();
        let preset = self.preset();
        self.overlay.update(preset, pos, self.app.visible);
    }

    /// 重载当前预设（激活/切换后），更新共享仓库与热键
    fn reload_preset(&mut self) {
        let preset: Arc<Preset> = {
            let store = self.store.lock();
            Arc::new(store.get_active().clone())
        };
        {
            let mut w = self.shared.write();
            w.preset = preset;
            w.version = w.version.wrapping_add(1);
        }
        self.last_version = self.shared.read().version;
        self.apply_hotkeys();
        self.sync_gamepad_cfg();
        self.full_update();
    }

    /// UI 进程经 WM_COPYDATA 推送的实时修改
    fn apply_ipc_payload(&mut self, json: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
            info!("IPC: JSON 解析失败, len={}", json.len());
            return;
        };
        let Some(pv) = v.get("p") else { return };
        let Ok(preset) = serde_json::from_value::<acaja::config::Preset>(pv.clone()) else {
            return;
        };
        let visible = v.get("v").and_then(|x| x.as_bool()).unwrap_or(self.app.visible);
        let preset = Arc::new(preset);
        info!("IPC: 收到 UI 推送 shape={:?} visible={}", preset.shape, visible);
        {
            let mut w = self.shared.write();
            w.preset = preset;
            w.version = w.version.wrapping_add(1);
        }
        self.last_version = self.shared.read().version;
        self.app.visible = visible;
        self.apply_hotkeys();
        self.sync_gamepad_cfg();
        self.full_update();
    }

    fn sync_gamepad_cfg(&mut self) {
        let p = self.preset();
        *self.gamepad_cfg.write() = RuntimeGamepadCfg::from_preset(&p);
    }

    fn apply_hotkeys(&mut self) {
        unreg_hotkey(self.hwnd, HOTKEY_ID_TOGGLE);
        unreg_hotkey(self.hwnd, HOTKEY_ID_NEXT_PRESET);
        let p = self.preset();
        if !p.hotkey_toggle.is_empty() {
            reg_hotkey(self.hwnd, HOTKEY_ID_TOGGLE, p.hotkey_toggle.modifiers, p.hotkey_toggle.vk);
        }
        if !p.hotkey_next_profile.is_empty() {
            reg_hotkey(self.hwnd, HOTKEY_ID_NEXT_PRESET, p.hotkey_next_profile.modifiers, p.hotkey_next_profile.vk);
        }
    }

    fn toggle_visible(&mut self) {
        self.app.visible = !self.app.visible;
        let preset = self.preset();
        self.overlay.update(preset, self.position_for_preset(), self.app.visible);
        info!("切换准星: visible={}", self.app.visible);
    }

    /// 监控配置文件变化（UI「应用」后自动重载，无需重启主程序）
    fn check_config_files(&mut self) -> bool {
        let appdata = match acaja::appdata_dir() {
            Ok(d) => d,
            Err(_) => return false,
        };
        let app_path = appdata.join("app.json");
        let preset_path = appdata.join("presets").join(format!("{}.json", self.watched.preset_name));
        let app_mtime = std::fs::metadata(&app_path).and_then(|m| m.modified()).ok();
        let preset_mtime = std::fs::metadata(&preset_path).and_then(|m| m.modified()).ok();

        // 预设名可能变了（app.json 的 last_preset）——先看 app.json
        if app_mtime != self.watched.app_mtime {
            self.watched.app_mtime = app_mtime;
            let name = { self.store.lock().active_name() };
            if name != self.watched.preset_name {
                self.watched.preset_name = name.clone();
                info!("配置监控: 激活预设切换为 {name}");
                // 重读 app.json 里的 last_preset 对应的预设文件（mtime 基线重置）
                let p2 = appdata.join("presets").join(format!("{name}.json"));
                self.watched.preset_mtime = std::fs::metadata(&p2).and_then(|m| m.modified()).ok();
                self.reload_preset();
                return true;
            }
        }
        if preset_mtime != self.watched.preset_mtime {
            self.watched.preset_mtime = preset_mtime;
            if let Some(mt) = preset_mtime {
                info!("配置监控: 预设文件已更新 ({:?})，重新应用", mt);
                self.apply_preset_file(&preset_path);
                return true;
            }
        }
        false
    }

    /// 直接读取预设文件并应用到准星（不依赖 IPC）
    fn apply_preset_file(&mut self, path: &std::path::Path) {
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let Ok(preset) = serde_json::from_str::<acaja::config::Preset>(&text) else {
            warn!("配置监控: 预设文件解析失败");
            return;
        };
        let preset = Arc::new(preset);
        {
            let mut w = self.shared.write();
            w.preset = preset.clone();
            w.version = w.version.wrapping_add(1);
        }
        self.last_version = self.shared.read().version;
        self.apply_hotkeys();
        self.sync_gamepad_cfg();
        self.full_update();
        info!("配置监控: 已应用新预设");
    }

    /// 命令文件（UI「退出主程序」）：返回 true = 应退出
    fn check_command_file(&mut self) -> bool {
        let Some(appdata) = acaja::appdata_dir().ok() else { return false };
        let cmd_path = appdata.join("cmd.json");
        if !cmd_path.exists() {
            return false;
        }
        let text = std::fs::read_to_string(&cmd_path).unwrap_or_default();
        let _ = std::fs::remove_file(&cmd_path);
        if text.contains("\"quit\"") || text.contains("quit") {
            info!("收到退出命令（UI 请求）");
            return true;
        }
        false
    }

    fn cycle_preset(&mut self) {
        let (names, current) = {
            let store = self.store.lock();
            (store.preset_names(), store.active_name())
        };
        let next = next_preset(&current, &names);
        if next != current {
            let ok = { self.store.lock().activate(&next) };
            if ok {
                info!("热键切到预设: {next}");
                self.reload_preset();
            }
        }
    }

    fn fire_started(&mut self, expand: f32) {
        if self.app.visible && expand > 0.0 {
            let preset = self.preset();
            self.overlay.update_with_expand(preset, self.position_for_preset(), true, expand);
        }
    }

    fn on_right_button(&mut self, down: bool) {
        let p = self.preset();
        if !p.right_click_toggle {
            return;
        }
        use acaja::config::RightClickMode::*;
        let want = match (p.right_click_mode, down) {
            (Click, true) => Some(!self.app.visible),
            (Click, false) => None,
            (HoldShow, true) => Some(true),
            (HoldShow, false) => Some(false),
            (HoldHide, true) => Some(false),
            (HoldHide, false) => Some(true),
        };
        if let Some(show) = want {
            if show != self.app.visible {
                self.app.visible = show;
                let preset = self.preset();
                self.overlay.update(preset, self.position_for_preset(), show);
                info!("右键切换: visible={show}");
            }
        }
    }

    fn on_gamepad(&mut self, ev: GameEvent) {
        let p = self.preset();
        let expand = p.dynamic.fire_expand_px;
        match ev {
            GameEvent::Ads(ads) => {
                if apply_ads_event(&mut self.app, p.gamepad.ads_mode, ads) {
                    let preset = self.preset();
                    self.overlay.update(preset, self.position_for_preset(), self.app.visible);
                    info!("手柄 ADS: ads={ads} → visible={}", self.app.visible);
                }
            }
            GameEvent::Fire(fire) => {
                if fire {
                    self.fire_started(expand);
                }
            }
        }
    }
}

// ===========================================================================
// Win32 基础设施
// ===========================================================================

unsafe extern "system" fn main_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn hinstance() -> HINSTANCE {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    unsafe { HINSTANCE(GetModuleHandleW(PCWSTR::null()).unwrap_or_default().0) }
}

fn create_message_window() -> HWND {
    unsafe {
        let wc = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(main_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance(),
            hIcon: windows::Win32::UI::WindowsAndMessaging::HICON(std::ptr::null_mut()),
            hCursor: windows::Win32::UI::WindowsAndMessaging::HCURSOR(std::ptr::null_mut()),
            hbrBackground: windows::Win32::Graphics::Gdi::HBRUSH(std::ptr::null_mut()),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: MAIN_CLASS,
        };
        RegisterClassW(&wc);
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            MAIN_CLASS,
            w!("ACAJABackend"),
            WINDOW_STYLE(WS_POPUP.0),
            0, 0, 1, 1,
            None, None, None, None,
        ) {
            Ok(h) => h,
            Err(e) => {
                warn!("消息窗口创建失败: {e}");
                HWND::default()
            }
        }
    }
}

/// 拉起设置 UI 进程（独立二进制 acaja-ui.exe；单例由 UI 进程自身互斥体保证）。
/// v1.1.4：文件名通配兼容——同目录下任何以 "acaja-ui" 开头的 .exe 都可以
/// （用户重命名如 ACAJA-UI-v1.1.4-x64.exe 也能自动找到）。
fn spawn_ui_process() {
    use std::path::{Path, PathBuf};
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mut candidates: Vec<PathBuf> = Vec::new();
    // 1) 同目录扫描通配
    if let Some(dir) = &exe_dir {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut hits: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()) == Some("exe".into())
                        && p.file_stem()
                            .map(|s| s.to_string_lossy().to_ascii_lowercase().starts_with("acaja-ui"))
                            .unwrap_or(false)
                })
                .collect();
            hits.sort();
            candidates.extend(hits);
        }
        candidates.push(dir.join("acaja-ui.exe"));
    }
    candidates.push(PathBuf::from("acaja-ui.exe"));
    // 主程序带 --diag 时把参数透传给设置进程：一条命令就能同时拿到两侧日志
    // （界面毛玻璃是否生效、系统模糊调用结果都在 acaja-ui-diag.log 里）
    let diag = std::env::args().any(|a| a == "--diag");
    for c in &candidates {
        if !Path::new(c).exists() {
            continue;
        }
        let mut cmd = std::process::Command::new(c);
        if diag {
            cmd.arg("--diag");
        }
        match cmd.spawn() {
            Ok(_) => {
                info!("设置进程已拉起: {}{}", c.display(), if diag { " (--diag)" } else { "" });
                return;
            }
            Err(e) => warn!("设置进程拉起失败（{}）: {e}", c.display()),
        }
    }
    warn!("未找到 acaja-ui.exe（查找过: {:?}）", candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());
    // 明确提示（发行版无日志，此提示是唯一反馈）。必须用独立线程弹：
    // 在主消息线程弹模态框会同时卡住托盘/热键/实时推送，用户现象就是「程序卡住」。
    show_message_box(
        "ACAJA 提示",
        "未找到设置程序 acaja-ui.exe\n\n请确认它与主程序放在同一目录，然后重试（托盘 → 打开设置）。",
    );
}

// ===========================================================================
// 事件处理（前台 / 消息泵）
// ===========================================================================

fn handle_fg_event(state: &mut MainState, fg: FgEvent) {
    match fg {
        FgEvent::Changed { exe, .. } => {
            state.fg_exe = exe.clone();
            let binding = {
                let store = state.store.lock();
                store
                    .app
                    .game_bindings
                    .iter()
                    .find(|b| b.exe.eq_ignore_ascii_case(&exe))
                    .map(|b| b.preset.clone())
            };
            if let Some(preset_name) = binding {
                let current = { state.store.lock().active_name() };
                if current != preset_name
                    && state.store.lock().activate(&preset_name)
                {
                    info!("前台 {exe} → 自动切换预设 {preset_name}");
                    state.reload_preset();
                }
            }
        }
        FgEvent::Moved { rect } => {
            let p = state.preset();
            if p.snap_to_window {
                let pos = snap_position(rect);
                state.overlay.move_to(pos);
            }
        }
    }
}

/// 消息泵批处理；返回 true = 应退出消息线程
fn pump_message_batch(state: &mut MainState, quit: &Arc<AtomicBool>) -> bool {
    let mut exit_now = false;
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                info!("收到 WM_QUIT");
                quit.store(true, Ordering::SeqCst);
                exit_now = true;
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);

            match msg.message {
                WM_HOTKEY => {
                    let id = (msg.wParam.0 as u32) & 0xFFFF;
                    if id == HOTKEY_ID_TOGGLE {
                        state.toggle_visible();
                    } else if id == HOTKEY_ID_NEXT_PRESET {
                        state.cycle_preset();
                    }
                }
                WM_TRAYICON => {
                    let what = (msg.lParam.0 as u32) & 0xFFFF;
                    info!("托盘消息: what={what:#x}");
                    if what == WM_LBUTTONDBLCLK {
                        state.toggle_visible();
                    } else if what == WM_RBUTTONUP || what == WM_CONTEXTMENU {
                        // 菜单文案跟随 app.json 的语言设置（此前写死中文）
                        let lang = state.store.lock().app.lang();
                        let labels = [
                            t(lang, "tray_toggle"),
                            t(lang, "tray_settings"),
                            t(lang, "tray_quit"),
                        ];
                        let mut pt = windows::Win32::Foundation::POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        let cmd = state
                            .tray
                            .as_mut()
                            .and_then(|t| t.popup_menu(pt.x, pt.y, state.hwnd, labels));
                        match cmd {
                            Some(CMD_TOGGLE) => state.toggle_visible(),
                            Some(CMD_SETTINGS) => spawn_ui_process(),
                            Some(CMD_QUIT) => {
                                info!("托盘退出");
                                quit.store(true, Ordering::SeqCst);
                                exit_now = true;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                WM_COPYDATA => {
                    info!("IPC: 收到 WM_COPYDATA");
                    // 设置进程推送的实时修改
                    let pcds = msg.lParam.0 as *const acaja::ipc::CopyDataStruct;
                    if !pcds.is_null() && (*pcds).dwData == IPC_TAG_PRESET {
                        let cds = &*pcds;
                        if cds.cbData > 0 && !cds.lpData.is_null() {
                            let bytes =
                                std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                            if let Ok(text) = std::str::from_utf8(bytes) {
                                state.apply_ipc_payload(text);
                            }
                        }
                    }
                }
                WM_INPUT => {
                    if let Some(ev) = handle_raw_input(msg.lParam) {
                        info!("RawInput: {ev:?}");
                        match ev {
                            RawMouseEvent::LeftDown => {
                                let expand = state.preset().dynamic.fire_expand_px;
                                state.fire_started(expand);
                            }
                            RawMouseEvent::RightDown => state.on_right_button(true),
                            RawMouseEvent::RightUp => state.on_right_button(false),
                            RawMouseEvent::LeftUp => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    exit_now
}

// ===========================================================================
// 后台壳进程
// ===========================================================================

fn init_logging() -> Option<PathBuf> {
    // --diag 诊断模式：发行版也写日志（acaja-diag.log）；平时保持零日志
    let diag = std::env::args().any(|a| a == "--diag");
    if !cfg!(debug_assertions) && !diag {
        return None;
    }
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let log_path = dir.join(if diag { "acaja-diag.log" } else { "acaja.log" });
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).ok()?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        file,
    )
    .ok()?;
    info!("日志文件：{}", log_path.display());
    Some(log_path)
}

/// 崩溃日志路径（`%APPDATA%/ACAJACrosshair/acaja-crash.log`）
fn crash_log_path() -> Option<PathBuf> {
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("acaja-crash.log"))
}

/// 最近一次 panic 的文本（catch_unwind 之后取出来提示用户）
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

/// 安装 panic 钩子：**任何** panic 都落盘。
///
/// 发行版（windows_subsystem = "windows" + 默认不写日志）里 panic 是「静默死亡」，
/// 用户只能看到「报错后直接卡掉」，开发者拿不到任何线索 —— 所以崩溃日志不走开关，
/// 恒定写入 `acaja-crash.log`。
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
        log::error!("{msg}");
        if let Some(path) = crash_log_path() {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                use std::io::Write;
                let _ = writeln!(f, "[{:?}] {msg}", std::time::SystemTime::now());
            }
        }
        *LAST_PANIC.lock() = Some(msg);
    }));
}

/// 弹窗提示：**必须**在独立线程里弹 —— 在主消息线程弹模态框会卡住托盘/热键/实时推送
fn show_message_box(title: &str, text: &str) {
    let t16: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let m16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    std::thread::spawn(move || unsafe {
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            None,
            windows::core::PCWSTR(m16.as_ptr()),
            windows::core::PCWSTR(t16.as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::MB_OK,
        );
    });
}

/// 消息循环的单次迭代（panic 隔离）：返回 (是否退出, 是否捕获到 panic)。
///
/// 一次消息处理 panic 不该让常驻准星进程消失；panic 详情已由 panic 钩子落盘，
/// 这里只保证主循环继续存活（否则用户看到的就是「准星突然没了」）。
fn run_message_iteration(state: &mut MainState) -> (bool, bool) {
    let quit = quit_flag();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pump_message_batch(state, &quit)
    })) {
        Ok(exit_now) => (exit_now, false),
        Err(_) => (false, true),
    }
}

/// 捕获 panic 后的一次性提示（只在首次弹窗，避免反复打扰）
fn report_caught_panic(count: u32) {
    warn!("消息循环捕获到内部错误（第 {count} 次），已记录到崩溃日志");
    if count > 1 {
        return;
    }
    let detail = LAST_PANIC.lock().clone().unwrap_or_default();
    let path = crash_log_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    show_message_box(
        "ACAJA 内部错误（已自动恢复）",
        &format!(
            "检测到内部错误，准星已自动恢复运行。\n\n{detail}\n\n详细日志：\n{path}\n\n如反复出现，请把该日志反馈给开发者。"
        ),
    );
}

fn backend_process_main() {
    init_logging();
    install_panic_hook();

    info!("{} v{} 后台壳启动", APP_NAME, VERSION);

    // ---- 单实例 ----
    let mutex: HANDLE =
        unsafe { CreateMutexW(None, false, w!("Local\\ACAJACrosshair_SingleInstance")) }
            .unwrap_or_default();
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        warn!("检测到已有实例在运行，本实例退出");
        return;
    }
    // 互斥体句柄故意不关闭：句柄是 Copy 类型、没有 Drop（进程退出时由系统回收），
    // 这里保留它即可让单实例判定持续有效。
    let _mutex_guard = mutex;

    // ---- 配置迁移 + 仓库 ----
    let appdata = match acaja::appdata_dir() {
        Ok(d) => d,
        Err(e) => {
            warn!("无法获取 APPDATA: {e}");
            return;
        }
    };
    let legacy_dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("CrosshairApp");
    match migrate_legacy(&legacy_dir, &appdata) {
        Ok(report) if report.presets > 0 => info!("旧版配置迁移完成：{} 个预设", report.presets),
        Ok(_) => {}
        Err(e) => warn!("旧版配置迁移失败: {e}"),
    }
    let store = match PresetStore::open(&appdata) {
        Ok(s) => s,
        Err(e) => {
            warn!("配置仓库打开失败: {e}");
            return;
        }
    };
    let store = Arc::new(Mutex::new(store));
    info!("当前预设：{}", store.lock().active_name());

    // ---- 覆盖层 ----
    let (overlay, overlay_thread) = acaja::overlay::start();

    // ---- 共享状态 ----
    let initial_preset = Arc::new(store.lock().get_active().clone());
    let shared: Arc<RwLock<SharedPreset>> = Arc::new(RwLock::new(SharedPreset {
        version: 1,
        preset: initial_preset.clone(),
    }));
    let gamepad_cfg: Arc<RwLock<RuntimeGamepadCfg>> =
        Arc::new(RwLock::new(RuntimeGamepadCfg::from_preset(&initial_preset)));

    let active_name = { store.lock().active_name() };
    let mut state = MainState {
        store,
        overlay: overlay.clone(),
        hwnd: create_message_window(),
        tray: None,
        app: AppState { visible: true, ..Default::default() },
        shared,
        last_version: 1,
        gamepad_cfg,
        fg_exe: String::new(),
        watched: FileWatch {
            app_mtime: None,
            preset_mtime: None,
            preset_name: active_name,
        },
    };

    // ---- 托盘 ----
    if state.hwnd.is_invalid() {
        // 消息窗口是一切交互（托盘/热键/RawInput/IPC）的宿主：失败必须让用户知道，
        // 否则表现为「准星在、但托盘热键全都没反应」
        warn!("消息窗口创建失败：托盘/热键/IPC 不可用");
        show_message_box(
            "ACAJA 启动异常",
            "消息窗口创建失败：托盘、热键与设置同步将不可用。\n\n请重启程序；若反复出现请反馈崩溃日志。",
        );
    }
    let icon_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("app.ico")))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    match Tray::add(state.hwnd, tray::TRAY_ID, &icon_path, "ACAJA 准星") {
        Ok(t) => {
            info!("托盘图标已创建");
            state.tray = Some(t);
        }
        Err(e) => warn!("托盘创建失败: {e}"),
    }

    state.apply_hotkeys();

    let (fg_tx, fg_rx): (Sender<FgEvent>, Receiver<FgEvent>) = unbounded();
    let fg_hook = start_fg_watcher(fg_tx);
    if fg_hook.is_some() {
        info!("前台检测已启动（自动切预设）");
    }

    if let Err(e) = register_raw_mouse(state.hwnd) {
        warn!("Raw Input 注册失败: {e}");
    }

    let _gamepad_watcher = start_gamepad(state.gamepad_cfg.clone(), Some(state.hwnd.0 as isize));
    let gamepad_rx = _gamepad_watcher.events.clone();

    // 初始显示 + 拉起设置进程（沿用「双击即见设置窗」的体验）
    state.full_update();
    spawn_ui_process();
    info!("后台壳就绪");

    // ---- 事件驱动主循环（v1.1.1：消息就绪即唤醒，空闲完全睡眠） ----
    // - 手柄线程有事件时 PostMessage(WAKE) 唤醒本线程（即时）
    // - WM_COPYDATA（UI 实时修改）SendMessage 进队 → MsgWait 立即唤醒
    // - 兜底 50ms 超时（消费 fg/其它通道）
    info!("主消息循环启动: hwnd={:?}", state.hwnd);
    let mut tick_counter: u32 = 0;
    let mut caught_panics: u32 = 0;
    loop {
        let (exit_now, panicked) = run_message_iteration(&mut state);
        if panicked {
            caught_panics = caught_panics.saturating_add(1);
            report_caught_panic(caught_panics);
        } else {
            caught_panics = 0;
        }
        if exit_now {
            break;
        }
        // 消费通道（前台事件）
        while let Ok(fg) = fg_rx.try_recv() {
            info!("前台事件: {:?}", fg);
            handle_fg_event(&mut state, fg);
        }
        // 消费手柄事件（已由 WAKE 消息唤醒，此处仅清空残余）
        while let Ok(ev) = gamepad_rx.try_recv() {
            info!("手柄事件: {:?}", ev);
            state.on_gamepad(ev);
        }
        // 阻塞等待：新消息 或 50ms 兜底
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MsgWaitForMultipleObjectsEx(
                None,
                50,
                windows::Win32::UI::WindowsAndMessaging::QS_ALLINPUT,
                windows::Win32::UI::WindowsAndMessaging::MWMO_INPUTAVAILABLE,
            );
        }
        tick_counter = tick_counter.wrapping_add(1);
        if tick_counter % 10 == 0 {
            // 每 ~0.5s：配置监控（UI「应用」按钮的兜底通道）
            if state.check_command_file() {
                break;
            }
            let _ = state.check_config_files();
        }
        if tick_counter % 400 == 0 {
            info!("心跳: 主循环存活");
        }
    }

    // ---- 清理 ----
    if let Some(h) = fg_hook {
        acaja::system::foreground::stop_fg_watcher(h);
    }
    if let Some(mut t) = state.tray.take() {
        t.remove();
    }
    overlay.close();
    if let Some(t) = overlay_thread {
        let _ = t.join();
    }
    info!("ACAJA 后台壳已退出");
}

/// 退出标志（主循环收尾用）
fn quit_flag() -> Arc<AtomicBool> {
    static Q: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    Q.get_or_init(|| Arc::new(AtomicBool::new(false))).clone()
}

// ===========================================================================
// 入口
// ===========================================================================

fn main() {
    backend_process_main();
}