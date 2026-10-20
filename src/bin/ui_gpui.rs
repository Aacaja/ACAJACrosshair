//! ACAJA 设置界面重构预览（GPUI Kit，`acaja-ui-gpui.exe`）。
//!
//! 与旧 `acaja-ui.exe` 并存：不替换默认启动路径，方便在 Windows 实机上对比。
//! 只依赖 `acaja::ui_gpui`（feature = "gpui-ui"）；后端 `acaja.exe` 不链接 GPUI。
//!
//! 运行：
//!   acaja-ui-gpui.exe            # 普通预览
//!   acaja-ui-gpui.exe --diag     # 额外把日志写到 acaja-ui-gpui.log（预览版默认就写）
//!   acaja-ui-gpui.exe --glass-test
//!
//! 关闭窗口 = 本进程退出；修改通过「应用设置到主程序」推给后台壳（WM_COPYDATA）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(windows, feature = "gpui-ui"))]
fn main() {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use acaja::config::PresetStore;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    let _log_path = init_logging();
    install_panic_hook();
    log::info!("ACAJA GPUI 设置预览启动");

    // 设置进程单例（与旧 egui 设置进程使用不同的名字，两者可以同时开着对比）
    let mutex: HANDLE =
        unsafe { CreateMutexW(None, false, w!("Local\\ACAJACrosshairSettingsGpui")) }
            .unwrap_or_default();
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        log::info!("已有 GPUI 设置预览在运行，退出");
        return;
    }
    let _mutex_guard = mutex;

    let appdata = match acaja::appdata_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("无法定位 APPDATA: {e}");
            return;
        }
    };
    let store = match PresetStore::open(&appdata) {
        Ok(store) => store,
        Err(e) => {
            log::error!("打开配置目录失败: {e}");
            return;
        }
    };

    let glass_test = std::env::args().any(|arg| arg == "--glass-test");
    acaja::ui_gpui::run(Arc::new(Mutex::new(store)), glass_test);
}

#[cfg(not(all(windows, feature = "gpui-ui")))]
fn main() {
    eprintln!("acaja-ui-gpui 需要 Windows + `gpui-ui` feature");
}

/// 预览版始终写日志：用户只需要把 `%APPDATA%/ACAJACrosshair/acaja-ui-gpui.log` 发回来。
fn init_logging() -> Option<std::path::PathBuf> {
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("acaja-ui-gpui.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        file,
    )
    .ok()?;
    log::info!("日志文件：{}", path.display());
    Some(path)
}

/// 与主程序/旧设置进程共用 `acaja-crash.log`；发行版 panic 必须可诊断。
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;

        let Ok(dir) = acaja::appdata_dir() else {
            return;
        };
        let path = dir.join("acaja-crash.log");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "[acaja-ui-gpui] {info}");
        }
    }));
}
