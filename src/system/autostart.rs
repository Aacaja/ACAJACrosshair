//! 开机自启（注册表 HKCU\...\Run）。
//!
//! 已核实签名（windows-rs 0.58）：
//! - `RegCreateKeyExW(hkey: P0, lpsubkey: P1, 0, lpclass: P2, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut hkey, None) -> WIN32_ERROR`（reg.rs:99）
//! - `RegSetValueExW(hkey, lpvaluename: P1, 0, REG_SZ, Some(&bytes)) -> WIN32_ERROR`（reg.rs:750）
//! - `RegDeleteValueW(hkey, lpvaluename: P1) -> WIN32_ERROR`（reg.rs:245）
//! - `RegCloseKey(hkey) -> WIN32_ERROR`（reg.rs:14）
//! - `HKEY_CURRENT_USER`、`KEY_WRITE`、`REG_OPTION_NON_VOLATILE`、`REG_SZ`（reg.rs:828/852/1691/1702）
//! - `ERROR_SUCCESS`（fnd.rs:4036）

use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::Foundation::ERROR_SUCCESS;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "ACAJACrosshair";
/// 写入自启项；enabled=false 时删除
pub fn set_autostart(enabled: bool) -> windows::core::Result<()> {
    let key = open_run_key()?;

    if enabled {
        let exe = std::env::current_exe().map_err(|e| windows::core::Error::from(e))?;
        let cmd = format!("\"{}\"", exe.display());
        // UTF-16LE 字节 + 结尾 \0
        let mut bytes: Vec<u8> = Vec::new();
        for u in cmd.encode_utf16().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let err = unsafe {
            RegSetValueExW(
                key,
                windows::core::w!("ACAJACrosshair"),
                0,
                REG_SZ,
                Some(&bytes),
            )
        };
        let _ = unsafe { RegCloseKey(key) };
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(windows::core::Error::from(err))
        }
    } else {
        let err = unsafe { RegDeleteValueW(key, windows::core::w!("ACAJACrosshair")) };
        let _ = unsafe { RegCloseKey(key) };
        // 值不存在也视为成功
        if err == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(windows::core::Error::from(err))
        }
    }
}

fn open_run_key() -> windows::core::Result<HKEY> {
    let mut key: HKEY = HKEY(std::ptr::null_mut());
    let err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if err == ERROR_SUCCESS {
        Ok(key)
    } else {
        Err(windows::core::Error::from(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_command_format() {
        let exe = r"C:\Program Files\ACAJA\acaja.exe";
        let cmd = format!("\"{}\"", exe);
        assert_eq!(cmd, "\"C:\\Program Files\\ACAJA\\acaja.exe\"");
        // UTF-16 字节流必须以 0 结尾（REG_SZ 要求）
        let mut bytes: Vec<u8> = Vec::new();
        for u in cmd.encode_utf16().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(bytes.len() % 2, 0);
        assert_eq!(&bytes[bytes.len() - 2..], &[0, 0]);
    }
}