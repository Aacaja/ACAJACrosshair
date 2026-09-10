//! 窗口外观增强（Windows 11）：系统圆角 + 亚克力/云母背板。
//!
//! 背景：设置窗口做成了「液态玻璃」视觉——半透明材质、镜面高光、悬浮层。界面本身自带
//! 全部玻璃绘制（不依赖系统能力），这里只是**锦上添花**：在 Win11 上再叠一层系统背板，
//! 让窗口背后的桌面内容真正透进来。
//!
//! 两个约束决定了这里的实现方式：
//! 1. eframe 的 `CreationContext` 不暴露 HWND → 按窗口标题查找本进程窗口（标题唯一）；
//! 2. 窗口创建与首帧之间有延迟 → `apply_once` 可每帧调用，窗口就绪后自动生效一次。
//!
//! 所有失败都静默降级（Win10 没有这些属性，返回 Err 属正常）。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

/// `DWMWA_WINDOW_CORNER_PREFERENCE`（Win11 起）；值 `DWMWCP_ROUND = 2`
const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
/// `DWMWA_SYSTEMBACKDROP_TYPE`（Win11 22H2 起）；值 `DWMSBT_TRANSIENTWINDOW = 3`（亚克力）
const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
const DWMWCP_ROUND: i32 = 2;
const DWMSBT_TRANSIENTWINDOW: i32 = 3;

/// 按标题查找窗口（本进程内标题唯一）
pub fn find_window_by_title(title: &str) -> Option<HWND> {
    let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { FindWindowW(None, windows::core::PCWSTR(wide.as_ptr())).ok() }
}

/// 应用系统圆角 + 亚克力背板；返回是否两项都成功
pub fn apply_liquid_glass(hwnd: HWND) -> bool {
    unsafe fn set_attr(hwnd: HWND, attr: u32, value: i32) -> bool {
        unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWINDOWATTRIBUTE(attr as i32),
                &value as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            )
            .is_ok()
        }
    }
    unsafe {
        let corner = set_attr(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND);
        let backdrop = set_attr(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_TRANSIENTWINDOW);
        corner && backdrop
    }
}

/// 首帧起可反复调用：窗口就绪后应用一次系统玻璃效果。
/// - `None` = 窗口还没建好（下次再试）或已经应用过
/// - `Some(ok)` = 已应用，`ok` 表示两项 DWM 属性是否都成功
pub fn apply_once(title: &str) -> Option<bool> {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.load(Ordering::SeqCst) {
        return None;
    }
    let hwnd = find_window_by_title(title)?;
    DONE.store(true, Ordering::SeqCst);
    Some(apply_liquid_glass(hwnd))
}
