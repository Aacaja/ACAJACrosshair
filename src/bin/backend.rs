//! ACAJA 入口（v1.1.0 双进程架构）。
//!
//! - 默认启动 = **后台壳进程**：D2D 准星 + 输入 + 托盘，内存 ~15MB；
//!   设置界面按需以独立进程 `acaja.exe --ui` 拉起，修改参数经 WM_COPYDATA 实时同步。
//! - `--ui` 参数 = **设置 UI 进程**：egui 界面；关闭即释放全部界面资源（~65MB）；
//!   每 2s 自检主进程窗口，主进程退出时自动退出。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use log::{info, warn};
use windows::Win32::Foundation::{GetLastError, HINSTANCE, HWND, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, PeekMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, WNDCLASSW, WNDCLASS_STYLES,
    MSG, PM_REMOVE, WM_CONTEXTMENU, WM_COPYDATA, WM_HOTKEY, WM_INPUT, WM_LBUTTONDBLCLK,
    WM_QUIT, WM_RBUTTONUP, WS_POPUP, WINDOW_EX_STYLE, WINDOW_STYLE,
};
use windows::core::{PCWSTR, w};

use acaja::config::{migrate_legacy, Preset, PresetStore};
use acaja::input::gamepad::{GameEvent, RuntimeGamepadCfg, start_gamepad};
use acaja::input::raw_mouse::{RawMouseEvent, handle_raw_input, register_raw_mouse};
use acaja::ipc::{self, BACKEND_WINDOW_TITLE, IPC_TAG_PRESET};
use acaja::overlay::OverlayHandle;
use acaja::state::{AppState, SharedPreset, apply_ads_event, next_preset, snap_position};
use acaja::system::foreground::{FgEvent, start_fg_watcher};
use acaja::system::hotkey::{HOTKEY_ID_NEXT_PRESET, HOTKEY_ID_TOGGLE, register as reg_hotkey, unregister as unreg_hotkey};
use acaja::system::tray::{self, CMD_QUIT, CMD_SETTINGS, CMD_TOGGLE, Tray, WM_TRAYICON};
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
}

impl MainState {
    fn preset(&self) -> Arc<Preset> {
        self.shared.read().unwrap().preset.clone()
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
            let store = self.store.lock().unwrap();
            Arc::new(store.get_active().clone())
        };
        {
            let mut w = self.shared.write().unwrap();
            w.preset = preset;
            w.version = w.version.wrapping_add(1);
        }
        self.last_version = { self.shared.read().unwrap().version };
        self.apply_hotkeys();
        self.sync_gamepad_cfg();
        self.full_update();
    }

    /// UI 进程经 WM_COPYDATA 推送的实时修改
    fn apply_ipc_payload(&mut self, json: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
            return;
        };
        let Some(pv) = v.get("p") else { return };
        let Ok(preset) = serde_json::from_value::<acaja::config::Preset>(pv.clone()) else {
            return;
        };
        let visible = v.get("v").and_then(|x| x.as_bool()).unwrap_or(self.app.visible);
        let preset = Arc::new(preset);
        {
            let mut w = self.shared.write().unwrap();
            w.preset = preset;
            w.version = w.version.wrapping_add(1);
        }
        self.last_version = { self.shared.read().unwrap().version };
        self.app.visible = visible;
        self.apply_hotkeys();
        self.sync_gamepad_cfg();
        self.full_update();
    }

    fn sync_gamepad_cfg(&mut self) {
        let p = self.preset();
        *self.gamepad_cfg.write().unwrap() = RuntimeGamepadCfg::from_preset(&p);
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

    fn cycle_preset(&mut self) {
        let (names, current) = {
            let store = self.store.lock().unwrap();
            (store.preset_names(), store.active_name())
        };
        let next = next_preset(&current, &names);
        if next != current {
            let ok = { self.store.lock().unwrap().activate(&next) };
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

/// 拉起设置 UI 进程（独立二进制 acaja-ui.exe；单例由 UI 进程自身互斥体保证）
fn spawn_ui_process() {
    use std::path::PathBuf;
    // 优先：同目录 acaja-ui.exe（双 exe 拆分，后端免链接 egui）
    let exe = std::env::current_exe().ok();
    let ui_exe = exe
        .as_ref()
        .map(|p| p.parent().map(|d| d.join("acaja-ui.exe")))
        .flatten()
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("acaja-ui.exe"));
    match std::process::Command::new(&ui_exe).spawn() {
        Ok(_) => info!("设置进程已拉起: {}", ui_exe.display()),
        Err(e) => warn!("设置进程拉起失败（{}）: {e}", ui_exe.display()),
    }
}

// ===========================================================================
// 事件处理（前台 / 消息泵）
// ===========================================================================

fn handle_fg_event(state: &mut MainState, fg: FgEvent) {
    match fg {
        FgEvent::Changed { exe, .. } => {
            state.fg_exe = exe.clone();
            let binding = {
                let store = state.store.lock().unwrap();
                store
                    .app
                    .game_bindings
                    .iter()
                    .find(|b| b.exe.eq_ignore_ascii_case(&exe))
                    .map(|b| b.preset.clone())
            };
            if let Some(preset_name) = binding {
                let current = { state.store.lock().unwrap().active_name() };
                if current != preset_name
                    && state.store.lock().unwrap().activate(&preset_name)
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
                    if what == WM_LBUTTONDBLCLK {
                        state.toggle_visible();
                    } else if what == WM_RBUTTONUP || what == WM_CONTEXTMENU {
                        let mut pt = windows::Win32::Foundation::POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        let cmd = state
                            .tray
                            .as_mut()
                            .and_then(|t| t.popup_menu(pt.x, pt.y, state.hwnd));
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
    if !cfg!(debug_assertions) {
        return None; // 发行版不生成日志文件
    }
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let log_path = dir.join("acaja.log");
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

fn backend_process_main() {
    init_logging();

    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
        log::error!("{msg}");
        eprintln!("{msg}");
    }));

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
    std::mem::forget(mutex);

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
    info!("当前预设：{}", store.lock().unwrap().active_name());

    // ---- 覆盖层 ----
    let (overlay, overlay_thread) = acaja::overlay::start();

    // ---- 共享状态 ----
    let initial_preset = Arc::new(store.lock().unwrap().get_active().clone());
    let shared: Arc<RwLock<SharedPreset>> = Arc::new(RwLock::new(SharedPreset {
        version: 1,
        preset: initial_preset.clone(),
    }));
    let gamepad_cfg: Arc<RwLock<RuntimeGamepadCfg>> =
        Arc::new(RwLock::new(RuntimeGamepadCfg::from_preset(&initial_preset)));

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
    };

    // ---- 托盘 ----
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

    let _gamepad_watcher = start_gamepad(state.gamepad_cfg.clone(), Some(state.hwnd));
    let gamepad_rx = _gamepad_watcher.events.clone();

    // 初始显示 + 拉起设置进程（沿用「双击即见设置窗」的体验）
    state.full_update();
    spawn_ui_process();
    info!("后台壳就绪");

    // ---- 事件驱动主循环（v1.1.1：消息就绪即唤醒，空闲完全睡眠） ----
    // - 手柄线程有事件时 PostMessage(WAKE) 唤醒本线程（即时）
    // - WM_COPYDATA（UI 实时修改）SendMessage 进队 → MsgWait 立即唤醒
    // - 兜底 50ms 超时（消费 fg/其它通道）
    loop {
        if pump_message_batch(&mut state, &quit_flag()) {
            break;
        }
        // 消费通道（前台事件）
        while let Ok(fg) = fg_rx.try_recv() {
            handle_fg_event(&mut state, fg);
        }
        // 消费手柄事件（已由 WAKE 消息唤醒，此处仅清空残余）
        while let Ok(ev) = gamepad_rx.try_recv() {
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
    }

    // ---- 清理 ----
    if let Some(h) = fg_hook {
        acaja::system::foreground::stop_fg_watcher(h);
    }
    if let Some(mut t) = state.tray.take() {
        t.remove();
    }
    overlay.close();
    let _ = overlay_thread.join();
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