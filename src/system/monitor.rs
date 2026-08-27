//! 多显示器枚举与坐标辅助。
//!
//! 已核实签名（windows-rs 0.58）：
//! - `EnumDisplayMonitors(hdc, lprcclip, lpfnenum: MONITORENUMPROC, dwdata) -> BOOL`（gdi.rs:720）
//! - `GetMonitorInfoW(hmonitor, &mut MONITORINFO) -> BOOL`（gdi.rs:1530）
//! - `MonitorFromPoint(POINT, MONITOR_FROM_FLAGS) -> HMONITOR`（gdi.rs:2105）

use std::sync::Once;

use log::warn;
use windows::Win32::Foundation::{BOOL, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MONITORENUMPROC, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, HDC, HMONITOR,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct MonitorInfo {
    /// 完整显示区域 (left, top, right, bottom)
    pub rect: (i32, i32, i32, i32),
    /// 工作区（不含任务栏）(left, top, right, bottom)
    pub work: (i32, i32, i32, i32),
    pub primary: bool,
}

impl MonitorInfo {
    /// 工作区中心（准星默认落点）
    pub fn work_center(&self) -> (i32, i32) {
        (self.work.0 + (self.work.2 - self.work.0) / 2, self.work.1 + (self.work.3 - self.work.1) / 2)
    }
    /// 显示区中心
    pub fn rect_center(&self) -> (i32, i32) {
        (self.rect.0 + (self.rect.2 - self.rect.0) / 2, self.rect.1 + (self.rect.3 - self.rect.1) / 2)
    }
    /// 点是否在该显示器内
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.rect.0 && x < self.rect.2 && y >= self.rect.1 && y < self.rect.3
    }
}

/// 全部显示器
pub fn monitors() -> Vec<MonitorInfo> {
    let mut list: Vec<MonitorInfo> = Vec::new();
    let capture = &mut list as *mut Vec<MonitorInfo>;

    unsafe extern "system" fn enum_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<MonitorInfo>) };
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
            list.push(MonitorInfo {
                rect: (mi.rcMonitor.left, mi.rcMonitor.top, mi.rcMonitor.right, mi.rcMonitor.bottom),
                work: (mi.rcWork.left, mi.rcWork.top, mi.rcWork.right, mi.rcWork.bottom),
                primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        let _ = hmon;
        BOOL(1)
    }

    let proc: MONITORENUMPROC = Some(enum_proc);
    let ok = unsafe {
        EnumDisplayMonitors(None, None, proc, LPARAM(capture as isize))
    };
    if !ok.as_bool() {
        warn!("EnumDisplayMonitors 失败");
    }
    list
}

/// 点所在显示器（默认最近的）
pub fn monitor_at(x: i32, y: i32) -> Option<MonitorInfo> {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        if hmon.is_invalid() {
            return None;
        }
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            Some(MonitorInfo {
                rect: (mi.rcMonitor.left, mi.rcMonitor.top, mi.rcMonitor.right, mi.rcMonitor.bottom),
                work: (mi.rcWork.left, mi.rcWork.top, mi.rcWork.right, mi.rcWork.bottom),
                primary: mi.dwFlags & MONITORINFOF_PRIMARY != 0,
            })
        } else {
            None
        }
    }
}

/// 主显示器（无则在列表中找 primary）
pub fn primary() -> Option<MonitorInfo> {
    let all = monitors();
    all.iter().find(|m| m.primary).copied().or_else(|| all.first().copied())
}

static LOG_ONCE: Once = Once::new();

/// 显示器索引 → 显示器信息（越界/异常时回退主屏，并只告警一次）
pub fn by_index(idx: i32) -> Option<MonitorInfo> {
    if idx < 0 {
        return primary();
    }
    let all = monitors();
    match all.get(idx as usize) {
        Some(m) => Some(*m),
        None => {
            LOG_ONCE.call_once(|| warn!("显示器索引 {idx} 不存在，回退主屏"));
            all.iter().find(|m| m.primary).copied()
        }
    }
}

/// Primary 显示器编号（用于 UI 下拉默认项）
pub fn primary_index() -> i32 {
    monitors().iter().position(|m| m.primary).map(|i| i as i32).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_geometry_helpers() {
        let m = MonitorInfo { rect: (0, 0, 1920, 1080), work: (0, 0, 1920, 1040), primary: true };
        assert_eq!(m.work_center(), (960, 520));
        assert_eq!(m.rect_center(), (960, 540));
        assert!(m.contains(100, 100));
        assert!(!m.contains(2000, 100));
    }
}