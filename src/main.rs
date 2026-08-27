//! ACAJA 入口：日志 → 单实例 → 配置迁移 → 全部子系统装配。
//!
//! 线程模型：
//! - 主线程：Win32 消息泵（热键 WM_HOTKEY / Raw Input WM_INPUT / 托盘回调）+ 事件分发
//! - 渲染线程：overlay（D2D）
//! - 手柄线程：XInput 轮询
//! - UI 线程：egui 设置窗口（关闭即退出程序）

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

// ===========================================================================
// 主状态
// ===========================================================================

struct MainState {
    store: Arc<Mutex<PresetStore>>,
    overlay: OverlayHandle,
    hwnd: HWND,
    tray: Option<Tray>,
    app: AppState,
    /// 当前生效预设
    preset: Arc<Preset>,
    /// 当前前台 exe（大写）
    fg_exe: String,
}

impl MainState {
    /// 计算准星位置（显示器/居中/自定义坐标）
    fn position_for_preset(&self) -> (i32, i32) {
        let p = &self.preset;
        match (p.position.x, p.position.y) {
            (acaja::config::PosVal::Px(x), acaja::config::PosVal::Px(y)) => (x as i32, y as i32),
            (acaja::config::PosVal::Px(x), _) => (x as i32, monitor::primary().map(|m| m.work_center().1).unwrap_or_else(|| acaja::overlay::primary_screen_center().1)),
            (_, acaja::config::PosVal::Px(y)) => (monitor::primary().map(|m| m.work_center().0).unwrap_or_else(|| acaja::overlay::primary_screen_center().0), y as i32),
            _ => {
                monitor::by_index(p.position.monitor)
                    .map(|m| m.work_center())
                    .unwrap_or_else(acaja::overlay::primary_screen_center)
            }
        }
    }

    fn full_update(&mut self) {
        let pos = self.position_for_preset();
        self.overlay.update(self.preset.clone(), pos, self.app.visible);
    }

    /// 重新加载当前预设（激活/切换后调用），并重注册热键
    fn reload_preset(&mut self) {
        let preset = { self.store.lock().unwrap().get_active().clone() };
        self.preset = Arc::new(preset);
        self.apply_hotkeys();
        self.full_update();
    }

    fn apply_hotkeys(&mut self) {
        unreg_hotkey(self.hwnd, HOTKEY_ID_TOGGLE);
        unreg_hotkey(self.hwnd, HOTKEY_ID_NEXT_PRESET);
        let p = &self.preset;
        if !p.hotkey_toggle.is_empty() {
            reg_hotkey(self.hwnd, HOTKEY_ID_TOGGLE, p.hotkey_toggle.modifiers, p.hotkey_toggle.vk);
        }
        if !p.hotkey_next_profile.is_empty() {
            reg_hotkey(self.hwnd, HOTKEY_ID_NEXT_PRESET, p.hotkey_next_profile.modifiers, p.hotkey_next_profile.vk);
        }
    }

    fn toggle_visible(&mut self) {
        self.app.visible = !self.app.visible;
        self.overlay.update(self.preset.clone(), self.position_for_preset(), self.app.visible);
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

    /// 开火 → 动态扩散一次性推给渲染层（overlay 内部自动衰减）
    fn fire_started(&mut self, expand: f32) {
        if self.app.visible && expand > 0.0 {
            self.overlay.update_with_expand(
                self.preset.clone(),
                self.position_for_preset(),
                true,
                expand,
            );
        }
    }

    /// 右键切换（点击/按住显示/按住隐藏）
    fn on_right_button(&mut self, down: bool) {
        if !self.preset.right_click_toggle {
            return;
        }
        use acaja::config::RightClickMode::*;
        let want = match (self.preset.right_click_mode, down) {
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
                self.overlay.update(self.preset.clone(), self.position_for_preset(), show);
                info!("右键切换: visible={show}");
            }
        }
    }

    /// 手柄事件
    fn on_gamepad(&mut self, ev: GameEvent) {
        let expand = self.preset.dynamic.fire_expand_px;
        match ev {
            GameEvent::Ads(ads) => {
                if apply_ads_event(&mut self.app, self.preset.gamepad.ads_mode, ads) {
                    self.overlay.update(self.preset.clone(), self.position_for_preset(), self.app.visible);
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
// UI 线程
// ===========================================================================

fn spawn_ui(store: Arc<Mutex<PresetStore>>, overlay: OverlayHandle, done_tx: Sender<()>) {
    std::thread::Builder::new()
        .name("acaja-ui".into())
        .spawn(move || {
            match acaja::ui::run(store, overlay, "ACAJA") {
                Ok(()) => info!("设置窗口已关闭"),
                Err(e) => warn!("设置窗口异常退出: {e}"),
            }
            let _ = done_tx.send(());
        })
        .expect("spawn ui thread");
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
    info!("{} v{} 启动", APP_NAME, VERSION);

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
    let legacy_dir = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("CrosshairApp");
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

    // ---- 主状态 ----
    let mut state = MainState {
        store: store.clone(),
        overlay: overlay.clone(),
        hwnd: create_message_window(),
        tray: None,
        app: AppState { visible: true, ..Default::default() },
        preset: Arc::new(store.lock().unwrap().get_active().clone()),
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
    let gamepad_rx = start_gamepad(state.preset.gamepad.trigger_threshold).events;

    // ---- UI 线程 ----
    let (done_tx, done_rx): (Sender<()>, Receiver<()>) = unbounded();
    spawn_ui(store.clone(), overlay.clone(), done_tx);

    // 初始显示
    state.full_update();

    // =======================================================================
    // 主消息泵
    // =======================================================================
    info!("主消息泵启动");
    'pump: loop {
        // UI 已关闭 → 主循环退出（UI 的退出按钮直接 exit(0)，此处兜底窗口关闭路径）
        if let Ok(()) = done_rx.try_recv() {
            info!("UI 已关闭，主循环退出");
            break 'pump;
        }

        // 消息泵（非阻塞，事件轮询统一走下方 try_recv 节拍）
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    info!("收到 WM_QUIT，主循环退出");
                    break 'pump;
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
                                Some(CMD_SETTINGS) => {}
                                Some(CMD_QUIT) => {
                                    info!("托盘退出");
                                    let _ = PostQuitMessage(0);
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
                    if state.preset.snap_to_window {
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

        // 空闲节拍
        std::thread::sleep(Duration::from_millis(10));
    }

    // ---- 清理 ----
    if let Some(mut t) = state.tray.take() {
        t.remove();
    }
    acaja::system::foreground::stop_fg_watcher(fg_hook);
    overlay.close();
    let _ = overlay_thread.join();
    info!("ACAJA 已退出");
}

// 供未来「打开设置」命令使用的占位（避免未使用警告噪音）
#[allow(dead_code)]
fn _unused() {
    let _ = CMD_SETTINGS;
}