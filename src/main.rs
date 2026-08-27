//! ACAJA 入口：日志 → 单实例 → 配置迁移 → 全部子系统装配。
//!
//! 线程模型（v1.0.1 重构：egui 必须在主线程创建窗口）：
//! - 主线程：egui 设置窗口（eframe/winit，OpenGL 上下文创建对线程有要求）
//! - 消息线程：Win32 消息泵（热键 WM_HOTKEY / Raw Input WM_INPUT / 托盘回调 / WinEvent 前台检测）
//! - 渲染线程：overlay（D2D）
//! - 手柄线程：XInput 轮询

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use log::{info, warn};
use windows::Win32::Foundation::{
    GetLastError, HINSTANCE, HWND, ERROR_ALREADY_EXISTS, HANDLE,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, PeekMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, WNDCLASSW, WNDCLASS_STYLES,
    MSG, PM_REMOVE, WM_CONTEXTMENU, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_QUIT, WS_POPUP,
    WINDOW_EX_STYLE, WINDOW_STYLE,
};
use windows::core::{PCWSTR, w};

use acaja::config::{migrate_legacy, Preset, PresetStore};
use acaja::input::gamepad::{GameEvent, start_gamepad};
use acaja::input::raw_mouse::register_raw_mouse;
use acaja::overlay::OverlayHandle;
use acaja::state::{AppState, apply_ads_event, next_preset, snap_position};
use acaja::system::foreground::{FgEvent, start_fg_watcher};
use acaja::system::hotkey::{HOTKEY_ID_NEXT_PRESET, HOTKEY_ID_TOGGLE, register as reg_hotkey, unregister as unreg_hotkey};
use acaja::system::monitor;
use acaja::system::tray::{self, CMD_QUIT, CMD_SETTINGS, CMD_TOGGLE, Tray, WM_TRAYICON};
use acaja::{APP_NAME, VERSION};

const SINGLE_INSTANCE_MUTEX: &str = "Local\\ACAJACrosshair_SingleInstance";
const MAIN_CLASS: PCWSTR = w!("ACAJAMainWindow");

/// egui Context 全局句柄（托盘退出时经它关闭设置窗口）
use acaja::ui::UI_CTX;

use acaja::state::SharedPreset;

// ===========================================================================
// 主状态（消息线程持有）
// ===========================================================================

struct MainState {
    store: Arc<Mutex<PresetStore>>,
    overlay: OverlayHandle,
    hwnd: HWND,
    tray: Option<Tray>,
    app: AppState,
    shared: Arc<RwLock<SharedPreset>>,
    last_version: u64,
    /// 当前前台 exe（大写，用于去重）
    fg_exe: String,
}

impl MainState {
    /// 读取当前生效预设（来自共享仓库，UI 的改动即时可见）
    fn preset(&self) -> Arc<Preset> {
        self.shared.read().unwrap().preset.clone()
    }

    fn position_for_preset(&self) -> (i32, i32) {
        let p = self.preset();
        match (p.position.x, p.position.y) {
            (acaja::config::PosVal::Px(x), acaja::config::PosVal::Px(y)) => (x as i32, y as i32),
            (acaja::config::PosVal::Px(x), _) => (
                x as i32,
                monitor::primary()
                    .map(|m| m.work_center().1)
                    .unwrap_or_else(|| acaja::overlay::primary_screen_center().1),
            ),
            (_, acaja::config::PosVal::Px(y)) => (
                monitor::primary()
                    .map(|m| m.work_center().0)
                    .unwrap_or_else(|| acaja::overlay::primary_screen_center().0),
                y as i32,
            ),
            _ => monitor::by_index(p.position.monitor)
                .map(|m| m.work_center())
                .unwrap_or_else(acaja::overlay::primary_screen_center),
        }
    }

    fn full_update(&mut self) {
        let pos = self.position_for_preset();
        let preset = self.preset();
        self.overlay.update(preset, pos, self.app.visible);
    }

    /// 重新加载当前预设（激活/切换后调用），更新共享仓库并重注册热键
    fn reload_preset(&mut self) {
        let preset: Arc<Preset> = {
            let store = self.store.lock().unwrap();
            Arc::new(store.get_active().clone())
        };
        let mut w = self.shared.write().unwrap();
        w.preset = preset;
        w.version = w.version.wrapping_add(1);
        drop(w);
        self.apply_hotkeys();
        self.full_update();
    }

    /// 检测共享 preset 是否被 UI 更新（版本号变化 → 重同步热键）
    fn sync_from_ui(&mut self) {
        let (version, preset) = {
            let r = self.shared.read().unwrap();
            (r.version, r.preset.clone())
        };
        if version != self.last_version {
            self.last_version = version;
            self.apply_hotkeys();
        }
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

/// 创建隐藏消息窗口（挂托盘回调/热键/Raw Input）
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
            w!("ACAJA"),
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

// ===========================================================================
// 消息线程（托盘/热键/前台检测/RawInput/手柄 全部在此）
// ===========================================================================

fn msg_thread_main(
    store: Arc<Mutex<PresetStore>>,
    overlay: OverlayHandle,
    stop: Arc<AtomicBool>,
    quit: Arc<AtomicBool>,
    reopen: Arc<AtomicBool>,
    shared: Arc<RwLock<SharedPreset>>,
) {
    info!("消息线程启动");

    let hwnd = create_message_window();
    let mut state = MainState {
        store,
        overlay: overlay.clone(),
        hwnd,
        tray: None,
        app: AppState { visible: true, ..Default::default() },
        shared,
        last_version: 0,
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

    // ---- 热键 ----
    state.apply_hotkeys();

    // ---- 前台检测 ----
    let (fg_tx, fg_rx): (Sender<FgEvent>, Receiver<FgEvent>) = unbounded();
    let fg_hook = start_fg_watcher(fg_tx);
    if fg_hook.is_some() {
        info!("前台检测已启动（自动切预设）");
    }

    // ---- Raw Input ----
    if let Err(e) = register_raw_mouse(state.hwnd) {
        warn!("Raw Input 注册失败: {e}");
    }

    // ---- 手柄 ----
    let p0 = state.preset();
    let _gamepad_watcher = start_gamepad(
        p0.gamepad.trigger_threshold,
        acaja::input::gamepad::AdsSource::from_config(p0.gamepad.ads_button),
    );
    let gamepad_rx = _gamepad_watcher.events.clone();

    // 初始显示
    state.full_update();
    info!("消息线程就绪，准星已显示");

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        state.sync_from_ui();

        // ---- 消息泵（非阻塞 + 事件轮询节拍） ----
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    info!("收到 WM_QUIT");
                    acaja::ui::QUIT_REQUESTED.store(true, Ordering::SeqCst);
                    if acaja::ui::UI_OPEN.load(Ordering::SeqCst) {
                        if let Some(ctx) = UI_CTX.get() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    } else {
                        quit.store(true, Ordering::SeqCst);
                    }
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
                        } else if what == WM_CONTEXTMENU {
                            let mut pt = windows::Win32::Foundation::POINT::default();
                            let _ = GetCursorPos(&mut pt);
                            let cmd = state
                                .tray
                                .as_mut()
                                .and_then(|t| t.popup_menu(pt.x, pt.y, state.hwnd));
                            match cmd {
                                Some(CMD_TOGGLE) => state.toggle_visible(),
                                Some(CMD_SETTINGS) => {
                                    if !acaja::ui::show_settings_window() {
                                        info!("托盘：重新拉起设置窗口");
                                        reopen.store(true, Ordering::SeqCst);
                                    }
                                }
                                Some(CMD_QUIT) => {
                                    info!("托盘退出");
                                    acaja::ui::QUIT_REQUESTED.store(true, Ordering::SeqCst);
                                    if acaja::ui::UI_OPEN.load(Ordering::SeqCst) {
                                        if let Some(ctx) = UI_CTX.get() {
                                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                        }
                                    } else {
                                        quit.store(true, Ordering::SeqCst);
                                    }
                                    break;
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // ---- 前台事件 ----
        while let Ok(fg) = fg_rx.try_recv() {
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

        // ---- 手柄事件 ----
        while let Ok(ev) = gamepad_rx.try_recv() {
            state.on_gamepad(ev);
        }

        // 空闲节拍：UI 打开时 10ms（界面响应），UI 关闭后 50ms（省电）
        let tick_ms = if acaja::ui::UI_OPEN.load(Ordering::SeqCst) { 10 } else { 50 };
        std::thread::sleep(Duration::from_millis(tick_ms));
    }

    // ---- 清理 ----
    if let Some(h) = fg_hook {
        acaja::system::foreground::stop_fg_watcher(h);
    }
    if let Some(mut t) = state.tray.take() {
        t.remove();
    }
    info!("消息线程退出");
    unsafe { PostQuitMessage(0) };
}

// ===========================================================================
// 入口
// ===========================================================================

fn init_logging() -> Option<PathBuf> {
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

fn main() {
    init_logging();

    // ---- 全局 panic 钩子：GUI 无控制台，panic 必须落日志 ----
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {info}");
        log::error!("{msg}");
        eprintln!("{msg}");
    }));

    info!("{} v{} 启动", APP_NAME, VERSION);

    // ---- 单实例 ----
    let mutex: HANDLE =
        unsafe { CreateMutexW(None, false, w!("Local\\ACAJACrosshair_SingleInstance")) }
            .unwrap_or_default();
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        warn!("检测到已有实例在运行，本实例退出");
        // 弹窗提示：避免用户以为"秒退"是 bug
        let title: Vec<u16> = "ACAJA 已在运行".encode_utf16().chain(std::iter::once(0)).collect();
        let msg: Vec<u16> =
            "ACAJA 已经在后台运行（请查看托盘图标或结束旧进程后再打开）。"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                windows::core::PCWSTR(msg.as_ptr()),
                windows::core::PCWSTR(title.as_ptr()),
                windows::Win32::UI::WindowsAndMessaging::MB_OK,
            );
        }
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

    // ---- 共享预设仓库（UI ↔ 消息线程同步，v1.0.4） ----
    let shared: Arc<RwLock<SharedPreset>> = Arc::new(RwLock::new(SharedPreset {
        version: 1,
        preset: Arc::new(store.lock().unwrap().get_active().clone()),
    }));

    // ---- 覆盖层 ----
    let (overlay, overlay_thread) = acaja::overlay::start();

    // ---- 消息线程（Win32 事件） ----
    let stop = Arc::new(AtomicBool::new(false));
    let quit = Arc::new(AtomicBool::new(false)); // UI 不可用时的全局退出信号（托盘触发）
    let reopen = Arc::new(AtomicBool::new(false));
    let msg_thread = std::thread::Builder::new()
        .name("acaja-msg".into())
        .spawn({
            let store = store.clone();
            let overlay = overlay.clone();
            let stop = stop.clone();
            let quit = quit.clone();
            let reopen = reopen.clone();
            let shared = shared.clone();
            move || msg_thread_main(store, overlay, stop, quit, reopen, shared)
        })
        .expect("spawn msg thread");

    // ---- 主线程 = egui 设置窗口（Windows 上必须主线程创建窗口） ----
    // 内存/CPU 优化（v1.0.5）：窗口关闭 = 真正关闭并释放全部 egui/GL 资源；
    // 托盘「打开设置」时按需重新创建。日常常驻仅保留 D2D 覆盖层 + 消息线程。
    info!("启动设置窗口……");
    'ui_loop: loop {
        let ui_overlay = overlay.clone();
        let ui_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            acaja::ui::run(store.clone(), ui_overlay, shared.clone(), "ACAJA")
        }));

        match ui_result {
            Ok(Ok(())) => info!("设置窗口已关闭（资源已释放）"),
            Ok(Err(e)) => {
                let text = format!(
                    "设置窗口启动失败（准星与托盘继续运行）。\n\n错误：{e}\n\n详情见 %APPDATA%/ACAJACrosshair/acaja.log",
                );
                warn!("{text}");
                message_box(&"ACAJA 提示".to_string(), &text);
            }
            Err(_) => {
                let text = "设置窗口发生内部错误（已捕获）。\n\n准星与托盘继续运行。\n详情见 %APPDATA%/ACAJACrosshair/acaja.log";
                warn!("{text}");
                message_box(&"ACAJA 提示".to_string(), &text);
            }
        }

        // 窗口已关闭/失败：挂起等待托盘「打开设置」(reopen) 或「退出」(quit)
        info!("设置窗口生命周期结束，等待托盘指令");
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if quit.load(Ordering::SeqCst) {
                break 'ui_loop;
            }
            if reopen.load(Ordering::SeqCst) {
                reopen.store(false, Ordering::SeqCst);
                info!("重新创建设置窗口");
                break;
            }
        }
    }

    // ---- 收尾 ----
    stop.store(true, Ordering::SeqCst);
    let _ = msg_thread.join();
    overlay.close();
    let _ = overlay_thread.join();
    info!("ACAJA 已退出");
}

/// 弹窗提示（UTF-16 构造，避免 w! 宏仅字面量的限制）
fn message_box(title: &str, text: &str) {
    let title_utf16: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let text_utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
            None,
            windows::core::PCWSTR(text_utf16.as_ptr()),
            windows::core::PCWSTR(title_utf16.as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::MB_OK,
        );
    }
}