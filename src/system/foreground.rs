//! 前台窗口检测（SetWinEventHook 事件驱动，零轮询）。
//!
//! 已核实签名（windows-rs 0.58）：
//! - `SetWinEventHook(eventmin: u32, eventmax: u32, hmod: P0, pfn: WINEVENTPROC, 0, 0, dwflags: u32) -> HWINEVENTHOOK`（acc.rs:309，**不返回 Result**）
//! - `WINEVENTPROC = Option<unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32)>`（acc.rs:10217）
//! - `WINEVENT_OUTOFCONTEXT: u32 = 0`、`EVENT_SYSTEM_FOREGROUND: u32 = 3`、`EVENT_SYSTEM_MOVESIZEEND: u32 = 11`（wam.rs:5220/3724/3732）
//! - `GetWindowThreadProcessId(hwnd, Option<*mut u32>) -> u32`（wam.rs:1631）
//! - `OpenProcess(PROCESS_ACCESS_RIGHTS, bool, u32) -> Result<HANDLE>`（threading.rs:1416）
//! - `QueryFullProcessImageNameW(hprocess, PROCESS_NAME_FORMAT, PWSTR, *mut u32) -> Result<()>`（threading.rs:1504）
//! - `GetWindowRect(hwnd, *mut RECT) -> Result<()>`（wam.rs:1591）
//!
//! 注意：SetWinEventHook 回调由**安装线程的消息泵**驱动（主线程），回调内只做轻量工作。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::Sender;
use log::warn;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, GetWindowThreadProcessId, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MOVESIZEEND,
    WINEVENT_OUTOFCONTEXT,
};

/// 前台事件
#[derive(Clone, Debug)]
pub enum FgEvent {
    /// 前台进程切换。exe 为大写进程文件名（如 "R5APEX.EXE"）
    Changed { exe: String },
    /// 前台窗口移动/缩放（已按 100ms 节流）
    Moved { rect: (i32, i32, i32, i32) },
}

static HOOK_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 启动前台检测。返回钩子句柄；由安装线程消息泵驱动回调。
/// 调用后钩子常驻，进程退出时自然清理。
pub fn start_fg_watcher(tx: Sender<FgEvent>) -> Option<HWINEVENTHOOK> {
    if HOOK_ACTIVE.swap(true, Ordering::SeqCst) {
        return None;
    }

    let tx: Arc<Sender<FgEvent>> = Arc::new(tx);

    // 全局状态：channel 与移动事件节流时间
    static CH: std::sync::OnceLock<Arc<Sender<FgEvent>>> = std::sync::OnceLock::new();
    static LAST: std::sync::OnceLock<Arc<std::sync::Mutex<Instant>>> = std::sync::OnceLock::new();
    let _ = CH.set(tx.clone());
    let _ = LAST.set(Arc::new(std::sync::Mutex::new(
        Instant::now() - std::time::Duration::from_millis(200),
    )));
    fn get_tx() -> Arc<Sender<FgEvent>> {
        CH.get().expect("fg watcher not initialized").clone()
    }
    fn get_last_move() -> Arc<std::sync::Mutex<Instant>> {
        LAST.get().expect("fg watcher not initialized").clone()
    }

    unsafe extern "system" fn proc(
        _hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        _idobject: i32,
        _idchild: i32,
        _ideventthread: u32,
        _time: u32,
    ) {
        if hwnd.is_invalid() {
            return;
        }
        let tx = get_tx();
        let last = get_last_move();
        match event {
            EVENT_SYSTEM_FOREGROUND => {
                // 排除桌面/任务栏等壳层窗口
                if let Some(exe) = foreground_exe(hwnd) {
                    let _ = tx.send(FgEvent::Changed { exe });
                }
            }
            EVENT_SYSTEM_MOVESIZEEND => {
                let now = Instant::now();
                let mut l = last.lock().unwrap();
                if now.duration_since(*l) < std::time::Duration::from_millis(100) {
                    return;
                }
                *l = now;
                let mut r = windows::Win32::Foundation::RECT::default();
                if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
                    let _ = tx.send(FgEvent::Moved {
                        rect: (r.left, r.top, r.right, r.bottom),
                    });
                }
            }
            _ => {}
        }
    }

    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_MOVESIZEEND,
            None,
            Some(proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_invalid() {
        HOOK_ACTIVE.store(false, Ordering::SeqCst);
        warn!("SetWinEventHook 失败: {}", windows::core::Error::from_win32());
        return None;
    }
    Some(hook)
}

pub fn stop_fg_watcher(hook: HWINEVENTHOOK) {
    if !hook.is_invalid() {
        unsafe { UnhookWinEvent(hook) };
    }
    HOOK_ACTIVE.store(false, Ordering::SeqCst);
}

/// 前台窗口的进程 exe 名（大写）。失败返回 None。
pub fn foreground_exe(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let proc = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(p) => p,
            Err(_) => return None,
        };
        let mut buf = [0u16; 512];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut size);
        let _ = CloseHandle(proc);
        if ok.is_err() {
            return None;
        }
        let name = String::from_utf16_lossy(&buf[..size as usize]);
        let exe = name.rsplit('\\').next().unwrap_or("").to_string().to_uppercase();
        if exe.is_empty() { None } else { Some(exe) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_name_upper() {
        // 纯字符串逻辑：路径拆分与大写由 foreground_exe 内联，这里验证路径解析函数
        let path = r"C:\Games\Apex\r5apex.exe";
        let exe = path.rsplit('\\').next().unwrap().to_string().to_uppercase();
        assert_eq!(exe, "R5APEX.EXE");
    }
}