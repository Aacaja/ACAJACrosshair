//! Raw Input 全局鼠标：左右键按下/抬起（后台也可接收，无需钩子）。
//!
//! 已核实签名（windows-rs 0.58）：
//! - `RegisterRawInputDevices(&[RAWINPUTDEVICE], cbsize) -> Result<()>`（uiinput.rs:70）
//! - `RAWINPUTDEVICE { usUsagePage: u16, usUsage: u16, dwFlags: RAWINPUTDEVICE_FLAGS, hwndTarget: HWND }`
//! - `RIDEV_INPUTSINK: RAWINPUTDEVICE_FLAGS = 256`（uiinput.rs:94）
//! - `GetRawInputData(hrawinput: P0, RID_INPUT, pdata: Option<*mut c_void>, pcbsize: *mut u32, cbsizeheader) -> u32`（uiinput.rs:36）
//! - `RAWINPUT { header: RAWINPUTHEADER, data: RAWINPUT_0(mouse: RAWMOUSE) }`（uiinput.rs:264）
//! - `RAWMOUSE { usFlags: MOUSE_STATE, ... }`（uiinput.rs:357）
//! - `RI_MOUSE_LEFT_BUTTON_DOWN/UP = 1/2, RIGHT_DOWN/UP = 4/8`（wam.rs:4570-4575）
//! - `WM_INPUT = 255`（wam.rs:5338）

use log::warn;
use windows::Win32::Foundation::{HRAWINPUT, LPARAM, HWND};
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICE_FLAGS,
    RAW_INPUT_DATA_COMMAND_FLAGS, RID_INPUT, RIDEV_INPUTSINK,
};
use windows::Win32::UI::WindowsAndMessaging::{RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP};

/// 鼠标按钮事件
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RawMouseEvent {
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
}

/// 注册全局鼠标监听（RIDEV_INPUTSINK：窗口在后台也能收到）。
/// 消息由宿主窗口的 WM_INPUT 到达，见 [`handle_raw_input`]。
pub fn register_raw_mouse(hwnd: HWND) -> windows::core::Result<()> {
    let device = [RAWINPUTDEVICE {
        usUsagePage: 0x01, // 通用桌面
        usUsage: 0x02,     // 鼠标
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    }];
    let size = std::mem::size_of::<RAWINPUTDEVICE>() as u32;
    unsafe { RegisterRawInputDevices(&device, size) }
}

/// 处理 WM_INPUT 消息（主线程消息泵调用），返回鼠标按钮事件。
pub fn handle_raw_input(lparam: LPARAM) -> Option<RawMouseEvent> {
    unsafe {
        // LPARAM 是 isize，HRAWINPUT 包装 *mut c_void
        let hraw = HRAWINPUT(lparam.0 as *mut std::ffi::c_void);

        // 两阶段读取：先取大小，再取数据
        let mut size: u32 = 0;
        let _ = GetRawInputData(hraw, RID_INPUT, None, &mut size, std::mem::size_of::<windows::Win32::UI::Input::RAWINPUTHEADER>() as u32);
        if size == 0 || size > 4096 {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let written = GetRawInputData(
            hraw,
            RID_INPUT,
            Some(buf.as_mut_ptr() as *mut std::ffi::c_void),
            &mut size,
            std::mem::size_of::<windows::Win32::UI::Input::RAWINPUTHEADER>() as u32,
        );
        if written == 0 {
            return None;
        }
        let raw = &*(buf.as_ptr() as *const RAWINPUT);
        // 只关心鼠标设备（RIM_TYPEMOUSE = 0）
        if raw.header.dwType != 0 {
            return None;
        }
        let flags = raw.data.mouse.usFlags.0;
        if flags & RI_MOUSE_LEFT_BUTTON_DOWN != 0 {
            return Some(RawMouseEvent::LeftDown);
        }
        if flags & RI_MOUSE_LEFT_BUTTON_UP != 0 {
            return Some(RawMouseEvent::LeftUp);
        }
        if flags & RI_MOUSE_RIGHT_BUTTON_DOWN != 0 {
            return Some(RawMouseEvent::RightDown);
        }
        if flags & RI_MOUSE_RIGHT_BUTTON_UP != 0 {
            return Some(RawMouseEvent::RightUp);
        }
        None
    }
}

/// 注销（窗口销毁时）——由注册失败清理用；进程退出自动释放。
pub fn unregister_raw_mouse(hwnd: HWND) {
    let device = [RAWINPUTDEVICE {
        usUsagePage: 0x01,
        usUsage: 0x02,
        dwFlags: RAWINPUTDEVICE_FLAGS(0),
        hwndTarget: hwnd,
    }];
    let size = std::mem::size_of::<RAWINPUTDEVICE>() as u32;
    let _ = unsafe { RegisterRawInputDevices(&device, size) };
}