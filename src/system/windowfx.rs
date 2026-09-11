//! 窗口外观增强：**真正的系统级毛玻璃**（亚克力 / 云母 / Aero 模糊）+ 圆角。
//!
//! ## 为什么之前「完全没有毛玻璃透感」（v1.2.1 复盘）
//! 1. 界面自己铺了一层 96% 不透明的黑底（`rgba(8,8,10,244)`）——桌面根本透不进来，
//!    玻璃面板只是在「我们自己的渐变」上做半透明，看起来就是一块平面；
//! 2. 无边框窗口（`decorations(false)`）**没有窗口边框区域**，Win11 的 `DWMWA_SYSTEMBACKDROP_TYPE`
//!    只作用于边框区 → 属性设了也看不见，必须先把 DWM 玻璃延伸到整个客户区
//!    （`DwmExtendFrameIntoClientArea`, margins = -1）；
//! 3. 窗口句柄是按标题 `FindWindowW` 找的，找不到就静默重试，**没有任何诊断**。
//!
//! ## 现在的做法
//! - 句柄：枚举本进程的顶层窗口（`EnumWindows` + 进程 id），不再依赖标题；
//! - 模糊：按「亚克力 → 云母/背板 → Aero 模糊」依次尝试，记录哪一种成功；
//! - 状态：成功后置位 [`blur_active`]，UI 据此决定用「透」还是「实」的底色
//!   （系统模糊不可用时界面仍然完整，只是改用不透明底）；
//! - 所有步骤都写日志（设置进程用 `--diag` 启动会落盘 `acaja-ui-diag.log`）。
//!
//! 注意：桌面模糊只能由系统合成器提供，任何 UI 技术（egui / Qt / 网页）都一样——
//! 网页里的 `backdrop-filter` 模糊的是「页面自己的背景」，不是桌面。

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
// 注意：MARGINS 类型在 Win32::UI::Controls（windows-rs 0.58），不是 Dwm 模块
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

/// 已成功应用系统模糊（UI 读这个决定底色透明度）
static BLUR_ACTIVE: AtomicBool = AtomicBool::new(false);
/// 已尝试过（无论成功失败，避免每帧重复尝试）
static APPLIED: AtomicBool = AtomicBool::new(false);

/// `DWMWA_WINDOW_CORNER_PREFERENCE`（Win11 起）：`DWMWCP_ROUND = 2`
const DWMWA_WINDOW_CORNER_PREFERENCE: i32 = 33;
/// `DWMWA_SYSTEMBACKDROP_TYPE`（Win11 22H2 起）：`DWMSBT_TRANSIENTWINDOW = 3`（亚克力）
const DWMWA_SYSTEMBACKDROP_TYPE: i32 = 38;
const DWMWCP_ROUND: i32 = 2;
const DWMSBT_TRANSIENTWINDOW: i32 = 3;

// ---- 未公开 API：SetWindowCompositionAttribute（Win10 1803+ 起被广泛使用，Aero/亚克力模糊的唯一实用入口）----
const WCA_ACCENT_POLICY: i32 = 19;
const ACCENT_ENABLE_BLURBEHIND: i32 = 3;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: i32 = 4;

#[repr(C)]
struct AccentPolicy {
    accent_state: i32,
    accent_flags: i32,
    gradient_color: u32, // ABGR
    animation_id: i32,
}

#[repr(C)]
struct WindowCompositionAttributeData {
    attribute: i32,
    data: *mut c_void,
    size_of_data: usize,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowCompositionAttribute(hwnd: HWND, data: *mut WindowCompositionAttributeData) -> BOOL;
}

/// 实际生效的模糊方式（写日志用）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlurKind {
    /// 亚克力（Win10 1803+ / Win11，观感最好）
    Acrylic,
    /// Win11 系统背板（云母/亚克力）
    Backdrop,
    /// Aero 模糊（老系统兜底）
    Aero,
    /// 都不行（Win7 无合成等）：界面改用不透明底
    None,
}

/// 系统模糊是否已生效（UI 每帧读；`false` 时用不透明底，界面依然完整）
pub fn blur_active() -> bool {
    BLUR_ACTIVE.load(Ordering::Relaxed)
}

/// 枚举本进程的可见顶层窗口（设置进程只有一个，不依赖标题，避免无边框窗口找不到）
fn own_window() -> Option<HWND> {
    struct Ctx {
        pid: u32,
        found: Option<HWND>,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = unsafe { &mut *(lparam.0 as *mut Ctx) };
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == ctx.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            // 有标题的优先（eframe 的主窗口一定设了标题）
            let len = unsafe { GetWindowTextLengthW(hwnd) };
            if len <= 0 {
                return BOOL(1); // 继续找
            }
            let mut buf = [0u16; 256];
            let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
            if n > 0 {
                ctx.found = Some(hwnd);
                return BOOL(0); // 找到即停
            }
        }
        BOOL(1)
    }
    let mut ctx = Ctx { pid: unsafe { GetCurrentProcessId() }, found: None };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.found
}

/// 亚克力模糊（主路径）：`SetWindowCompositionAttribute` + ABGR 底色
fn try_acrylic(hwnd: HWND, state: i32, tint_abgr: u32) -> bool {
    let mut policy = AccentPolicy {
        accent_state: state,
        accent_flags: 0,
        gradient_color: tint_abgr,
        animation_id: 0,
    };
    let mut data = WindowCompositionAttributeData {
        attribute: WCA_ACCENT_POLICY,
        data: &mut policy as *mut AccentPolicy as *mut c_void,
        size_of_data: std::mem::size_of::<AccentPolicy>(),
    };
    unsafe { SetWindowCompositionAttribute(hwnd, &mut data).as_bool() }
}

/// Win11 背板 + 圆角；**必须同时把玻璃延伸到客户区**，否则无边框窗口看不到效果
fn try_backdrop(hwnd: HWND) -> bool {
    unsafe {
        let corner = DWMWCP_ROUND;
        let corner_ok = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(DWMWA_WINDOW_CORNER_PREFERENCE),
            &corner as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        )
        .is_ok();
        let backdrop = DWMSBT_TRANSIENTWINDOW;
        let backdrop_ok = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(DWMWA_SYSTEMBACKDROP_TYPE),
            &backdrop as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        )
        .is_ok();
        // 无边框窗口没有边框区：把 DWM 玻璃铺满客户区，背板才可见
        let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        let extend_ok = DwmExtendFrameIntoClientArea(hwnd, &margins).is_ok();
        corner_ok && backdrop_ok && extend_ok
    }
}

/// 立即应用（可重复调用；成功后置 [`blur_active`]）。返回实际生效的方式。
pub fn apply(hwnd: HWND) -> BlurKind {
    // 1) 亚克力（观感最好，Win10 1803+ / Win11）——底色偏暗蓝，配合界面的深黑玻璃
    if try_acrylic(hwnd, ACCENT_ENABLE_ACRYLICBLURBEHIND, 0x99_14_14_12) {
        log::info!("窗口效果: 亚克力模糊已启用");
        BLUR_ACTIVE.store(true, Ordering::Relaxed);
        return BlurKind::Acrylic;
    }
    // 2) Win11 系统背板（含圆角）
    if try_backdrop(hwnd) {
        log::info!("窗口效果: Win11 系统背板(亚克力)+圆角已启用");
        BLUR_ACTIVE.store(true, Ordering::Relaxed);
        return BlurKind::Backdrop;
    }
    // 3) Aero 模糊（老系统兜底）
    if try_acrylic(hwnd, ACCENT_ENABLE_BLURBEHIND, 0) {
        log::info!("窗口效果: Aero 模糊已启用（老系统兜底）");
        BLUR_ACTIVE.store(true, Ordering::Relaxed);
        return BlurKind::Aero;
    }
    log::warn!("窗口效果: 系统模糊不可用（Win7 无合成或调用被拒绝），界面改用不透明底");
    BlurKind::None
}

/// 每帧调用：窗口就绪后应用一次；返回 `Some(结果)` 表示这一次刚完成尝试。
///
/// 与旧版的区别：窗口用**本进程枚举**查找（不依赖标题、不依赖是否无边框），
/// 找到之前返回 `None`（下帧再试），尝试过之后恒返回 `None`（结果记录在 [`blur_active`]）。
pub fn apply_once() -> Option<BlurKind> {
    if APPLIED.load(Ordering::Relaxed) {
        return None;
    }
    let hwnd = own_window()?;
    APPLIED.store(true, Ordering::Relaxed);
    Some(apply(hwnd))
}

/// 诊断用：把当前状态写成一行（`--diag` 时由设置进程记录）
pub fn status_line() -> String {
    format!(
        "系统模糊={} 已尝试={}",
        if blur_active() { "生效" } else { "不可用" },
        APPLIED.load(Ordering::Relaxed)
    )
}
