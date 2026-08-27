//! 系统托盘图标（Shell_NotifyIcon）。
//!
//! 已核实签名（windows-rs 0.58）：
//! - `Shell_NotifyIconW(dwmessage: NOTIFY_ICON_MESSAGE, *const NOTIFYICONDATAW) -> BOOL`（shell.rs:4685）
//! - `NOTIFYICONDATAW { cbSize, hWnd, uID, uFlags, uCallbackMessage, hIcon, szTip: [u16;128], ... }`
//! - `LoadImageW(hinst, name, IMAGE_ICON(GDI_IMAGE_TYPE(1)), 0, 0, LR_LOADFROMFILE|LR_DEFAULTSIZE) -> Result<HANDLE>`（wam.rs:1975）
//! - `CreatePopupMenu() -> Result<HMENU>`、`AppendMenuW(hmenu, MENU_ITEM_FLAGS, usize, PCWSTR) -> Result<()>`
//! - `TrackPopupMenu(hmenu, TPM_RETURNCMD, x, y, 0, hwnd, None) -> BOOL`（wam.rs:3186）

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::{HBITMAP, HDC};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NOTIFY_ICON_DATA_FLAGS, NOTIFY_ICON_MESSAGE, NIF_ICON,
    NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, LoadImageW, TrackPopupMenu, HICON, IMAGE_ICON,
    LR_DEFAULTSIZE, LR_LOADFROMFILE, MF_STRING, TPM_RETURNCMD,
};

/// 托盘回调消息（挂在宿主窗口上）
pub const WM_TRAYICON: u32 = 0x8000 + 64; // WM_APP + 64
/// 托盘菜单命令回投消息（wParam = 命令 id）
pub const WM_TRAY_COMMAND: u32 = 0x8000 + 65;
pub const TRAY_ID: u32 = 1;

/// 托盘菜单命令
pub const CMD_TOGGLE: usize = 1001;
pub const CMD_SETTINGS: usize = 1002;
pub const CMD_QUIT: usize = 1003;

pub struct Tray {
    data: NOTIFYICONDATAW,
    icon: Option<HICON>,
}


impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
    }
}

impl Tray {
    /// 添加托盘图标。icon_path_ico 加载失败时回退到自绘红色十字。
    pub fn add(hwnd: HWND, u_id: u32, icon_path_ico: &str, tooltip: &str) -> windows::core::Result<Tray> {
        let icon = load_icon(icon_path_ico);

        let mut tip: [u16; 128] = [0; 128];
        let chars: Vec<u16> = tooltip.encode_utf16().take(127).collect();
        tip[..chars.len()].copy_from_slice(&chars);

        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: u_id,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAYICON,
            hIcon: icon.unwrap_or_default(),
            szTip: tip,
            ..Default::default()
        };
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };
        if !ok.as_bool() {
            return Err(windows::core::Error::from_win32());
        }
        Ok(Tray { data, icon })
    }

    pub fn set_tooltip(&mut self, tooltip: &str) -> windows::core::Result<()> {
        let mut tip: [u16; 128] = [0; 128];
        let chars: Vec<u16> = tooltip.encode_utf16().take(127).collect();
        tip[..chars.len()].copy_from_slice(&chars);
        self.data.szTip = tip;
        let ok = unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.data) };
        if !ok.as_bool() {
            Err(windows::core::Error::from_win32())
        } else {
            Ok(())
        }
    }

    pub fn remove(&mut self) {
        if self.data.uID != 0 {
            let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.data) };
            self.data.uID = 0;
        }
        if let Some(icon) = self.icon.take() {
            let _ = unsafe { windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(icon.0)) };
        }
    }

    /// 弹出托盘右键菜单（在宿主窗口消息线程调用）。返回用户选择的命令 id。
    pub fn popup_menu(&mut self, anchor_x: i32, anchor_y: i32, hwnd: HWND) -> Option<usize> {
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            let items = [
                (CMD_TOGGLE, "显示/隐藏准星\0"),
                (CMD_SETTINGS, "打开设置\0"),
                (CMD_QUIT, "退出\0"),
            ];
            for (id, text) in items {
                let wide: Vec<u16> = text.encode_utf16().collect();
                let _ = AppendMenuW(menu, MF_STRING, id, windows::core::PCWSTR(wide.as_ptr()));
            }
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD, anchor_x, anchor_y, 0, hwnd, None);
            let _ = DestroyMenu(menu);
            if cmd.0 == 0 {
                None
            } else {
                Some(cmd.0 as usize)
            }
        }
    }
}

/// 加载图标：优先 exe 内嵌资源（品牌 A 图标），其次 .ico 文件，最后自绘字母 A。
fn load_icon(path: &str) -> Option<HICON> {
    // 1) exe 资源图标（winres 植入的第一个图标，资源 ID = 1）
    let hinst = hinstance();
    if let Ok(h) = unsafe {
        LoadImageW(
            hinst,
            windows::core::PCWSTR(1 as *const u16),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE,
        )
    } {
        return Some(HICON(h.0));
    }
    // 2) .ico 文件
    let mut wide: Vec<u16> = path.encode_utf16().collect();
    wide.push(0);
    if let Ok(h) = unsafe {
        LoadImageW(
            None,
            windows::core::PCWSTR(wide.as_ptr()),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
    } {
        return Some(HICON(h.0));
    }
    // 3) 自绘字母 A
    draw_a_icon()
}

/// 当前模块实例句柄
fn hinstance() -> HINSTANCE {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    unsafe { HINSTANCE(GetModuleHandleW(windows::core::PCWSTR::null()).unwrap_or_default().0) }
}

/// 自绘 32x32 字母 A 图标（透明底白色 A，任务栏深色下清晰）
fn draw_a_icon() -> Option<HICON> {
    use windows::Win32::Graphics::Gdi::{CreateDIBSection, DeleteObject, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, HGDIOBJ};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = 32;
        bmi.bmiHeader.biHeight = -32;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let mut bits_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbmp = CreateDIBSection(
            HDC(std::ptr::null_mut()),
            &bmi,
            DIB_RGB_COLORS,
            &mut bits_ptr,
            HANDLE(std::ptr::null_mut()),
            0,
        )
        .ok()?;

        // 字母 A：顶点 (16,4) 左底 (7,26) 右底 (25,26)；横杠两段（中空缺口）
        let px = bits_ptr as *mut u32;
        let dist_seg = |pxx: f32, pyy: f32, x1: f32, y1: f32, x2: f32, y2: f32| -> f32 {
            let dx = x2 - x1;
            let dy = y2 - y1;
            let len2 = dx * dx + dy * dy;
            let t = (((pxx - x1) * dx + (pyy - y1) * dy) / len2).clamp(0.0, 1.0);
            let cx = x1 + t * dx;
            let cy = y1 + t * dy;
            ((pxx - cx).powi(2) + (pyy - cy).powi(2)).sqrt()
        };
        for y in 0..32i32 {
            for x in 0..32i32 {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let stroke = |a: (f32, f32, f32, f32)| dist_seg(fx, fy, a.0, a.1, a.2, a.3) < 2.3;
                let on = stroke((16.0, 4.0, 7.0, 26.0))
                    || stroke((16.0, 4.0, 25.0, 26.0))
                    || (fx > 8.0 && fx < 15.0 && stroke((8.0, 20.0, 24.0, 20.0)))
                    || (fx > 16.0 && fx < 24.0 && stroke((8.0, 20.0, 24.0, 20.0)));
                let v: u32 = if on { 0xFFFFFFFF } else { 0x00000000 }; // BGRA 白
                *px.add((y * 32 + x) as usize) = v;
            }
        }

        let mut mask_bmi: BITMAPINFO = std::mem::zeroed();
        mask_bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        mask_bmi.bmiHeader.biWidth = 32;
        mask_bmi.bmiHeader.biHeight = -32;
        mask_bmi.bmiHeader.biPlanes = 1;
        mask_bmi.bmiHeader.biBitCount = 1;
        mask_bmi.bmiHeader.biCompression = BI_RGB.0;
        let mut mask_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let hbmp_mask = CreateDIBSection(
            HDC(std::ptr::null_mut()),
            &mask_bmi,
            DIB_RGB_COLORS,
            &mut mask_ptr,
            HANDLE(std::ptr::null_mut()),
            0,
        )
        .ok()?;

        let info = ICONINFO {
            fIcon: windows::Win32::Foundation::BOOL(1),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: HBITMAP(hbmp_mask.0),
            hbmColor: hbmp,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(HGDIOBJ(hbmp.0));
        let _ = DeleteObject(HGDIOBJ(hbmp_mask.0));
        icon
    }
}

