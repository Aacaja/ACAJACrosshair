//! 主进程（后台壳）↔ 设置 UI 进程间通信（WM_COPYDATA）。
//!
//! 架构（v1.1.0）：设置界面拆分为独立进程（`acaja.exe --ui`），
//! 主进程只保留准星渲染 + 输入 + 托盘（内存 ~15MB）。
//! - UI 进程修改任何参数 → 组装 JSON → SendMessage(WM_COPYDATA) 同步推给主进程
//! - 主进程消息窗口收到后更新共享 preset / 热键 / 手柄配置 / 覆盖层
//! - UI 进程 2 秒自检主进程窗口：主进程退出则 UI 自动退出

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageW, WM_COPYDATA};

/// 主进程消息窗口标题（唯一标识，FindWindow 依据）
pub const BACKEND_WINDOW_TITLE: &str = "ACAJABackend";

/// 数据标记：完整预设 + 可见性
pub const IPC_TAG_PRESET: usize = 0xACAA_0001;

/// WM_COPYDATA 数据结构（windows-rs 未绑定，手工定义，布局与 winuser.h 一致）
#[repr(C)]
pub struct CopyDataStruct {
    pub dwData: usize,
    pub cbData: u32,
    pub lpData: *const std::ffi::c_void,
}

/// 查找主进程消息窗口
pub fn find_backend() -> Option<HWND> {
    let title: Vec<u16> = BACKEND_WINDOW_TITLE
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe { FindWindowW(None, windows::core::PCWSTR(title.as_ptr())).ok() }
}

/// 同步发送 JSON 负载到主进程（发送方阻塞至接收方处理完；负载很小，µs 级）
pub fn send_json(hwnd: HWND, tag: usize, json: &str) -> bool {
    let bytes = json.as_bytes();
    let cds = CopyDataStruct {
        dwData: tag,
        cbData: bytes.len() as u32,
        lpData: bytes.as_ptr() as *const std::ffi::c_void,
    };
    let _ = unsafe {
        SendMessageW(
            hwnd,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&cds as *const CopyDataStruct as isize),
        )
    };
    true
}

/// 组装「预设+可见性」负载 JSON
pub fn preset_payload(preset: &crate::config::Preset, visible: bool) -> String {
    let value = serde_json::json!({
        "v": visible,
        "p": serde_json::to_value(preset).unwrap_or(serde_json::Value::Null),
    });
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_roundtrip() {
        let p = crate::config::Preset::default();
        let json = preset_payload(&p, true);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["v"], true);
        let back: crate::config::Preset = serde_json::from_value(v["p"].clone()).unwrap();
        assert_eq!(back, p);
    }
}