//! 全局热键（RegisterHotKey）。
//!
//! 已核实签名（windows-rs 0.58）：
//! - `RegisterHotKeyW(hwnd, id, fsmodifiers: HOT_KEY_MODIFIERS, vk) -> Result<()>`（wam.rs）
//! - `UnregisterHotKey(hwnd, id) -> Result<()>`
//! - `WM_HOTKEY = 786`（wam.rs:5318）

use log::info;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::Foundation::HWND;

pub const HOTKEY_ID_TOGGLE: u32 = 1;
pub const HOTKEY_ID_NEXT_PRESET: u32 = 2;

/// 热键修饰键（与 config 的 MOD_* 位标志一致，映射为 Win32 类型）
pub fn modifier_flags(mods: u32) -> HOT_KEY_MODIFIERS {
    let mut f = HOT_KEY_MODIFIERS(0);
    if mods & crate::config::MOD_CONTROL != 0 {
        f |= MOD_CONTROL;
    }
    if mods & crate::config::MOD_ALT != 0 {
        f |= MOD_ALT;
    }
    if mods & crate::config::MOD_SHIFT != 0 {
        f |= MOD_SHIFT;
    }
    if mods & crate::config::MOD_WIN != 0 {
        f |= MOD_WIN;
    }
    f
}

/// 注册热键；成功返回 true。id 冲突时先注销再注册。
pub fn register(hwnd: HWND, id: u32, mods: u32, vk: u32) -> bool {
    if vk == 0 {
        return false;
    }
    // RegisterHotKey 对重复注册返回 ERROR_HOTKEY_ALREADY_REGISTERED，先注销一次
    let _ = unsafe { UnregisterHotKey(hwnd, id as i32) };
    match unsafe { RegisterHotKey(hwnd, id as i32, modifier_flags(mods), vk) } {
        Ok(()) => {
            info!("热键已注册 id={id} vk={vk} mods={mods:#x}");
            true
        }
        Err(e) => {
            log::warn!("热键注册失败 id={id}: {e}");
            false
        }
    }
}

pub fn unregister(hwnd: HWND, id: u32) {
    let _ = unsafe { UnregisterHotKey(hwnd, id as i32) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_mapping() {
        assert_eq!(modifier_flags(crate::config::MOD_CONTROL).0, MOD_CONTROL.0);
        assert_eq!(modifier_flags(crate::config::MOD_CONTROL | crate::config::MOD_ALT).0,
                   (MOD_CONTROL | MOD_ALT).0);
        assert_eq!(modifier_flags(crate::config::MOD_SHIFT | crate::config::MOD_WIN).0,
                   (MOD_SHIFT | MOD_WIN).0);
        assert_eq!(modifier_flags(0).0, 0);
    }
}