//! ACAJA 设置 UI 进程入口（`acaja-ui.exe`，由后台壳按需拉起）。
//!
//! 独立进程 = 用完即走：窗口关闭 → 本进程退出 → 全部 egui/GL 资源释放。
//! 参数经 WM_COPYDATA 实时同步给主进程（覆盖层/热键/手柄即时生效）。
//! 每 2s 自检主进程窗口，主进程退出/崩溃 → 自动退出。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use log::{info, warn};
use windows::Win32::Foundation::{GetLastError, HANDLE, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::w;

use acaja::config::PresetStore;
use acaja::ipc;

fn init_logging() -> Option<PathBuf> {
    let diag = std::env::args().any(|a| a == "--diag");
    if !cfg!(debug_assertions) && !diag {
        return None; // 发行版默认不生成日志；--diag 诊断模式例外
    }
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let log_path = dir.join(if diag { "acaja-ui-diag.log" } else { "acaja-ui.log" });
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
    info!("ACAJA 设置进程启动");

    // 设置进程单例
    let mutex: HANDLE =
        unsafe { CreateMutexW(None, false, w!("Local\\ACAJACrosshairSettings")) }
            .unwrap_or_default();
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        warn!("设置进程已在运行");
        return;
    }
    std::mem::forget(mutex);

    // 主进程自检：主进程消失（退出/崩溃）→ 设置进程兜底退出
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if ipc::find_backend().is_none() {
            std::process::exit(0);
        }
    });

    let appdata = match acaja::appdata_dir() {
        Ok(d) => d,
        Err(e) => {
            warn!("无法获取 APPDATA: {e}");
            return;
        }
    };
    let store = match PresetStore::open(&appdata) {
        Ok(s) => s,
        Err(e) => {
            warn!("配置仓库打开失败: {e}");
            return;
        }
    };
    let store = Arc::new(Mutex::new(store));

    match acaja::ui::run(store, "ACAJA") {
        Ok(()) => info!("设置进程正常退出"),
        Err(e) => warn!("设置进程异常: {e}"),
    }
}
