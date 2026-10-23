//! ACAJA 设置进程（`acaja-ui.exe`，GPUI Kit）。
//!
//! 独立进程 = 用完即走：窗口关闭 → 本进程退出 → GPUI/GPU 资源释放。
//! 参数经 WM_COPYDATA 实时同步给主进程（覆盖层/热键/手柄即时生效）。
//! 每 2s 自检主进程窗口，主进程退出/崩溃 → 自动退出；`--standalone` 可跳过自检
//! （开发时在没有后台壳的情况下单独预览界面）。
//!
//! 参数：
//!   --diag        写诊断日志 `%APPDATA%/ACAJACrosshair/acaja-ui-diag.log`
//!   --standalone  不因主进程缺失而退出（仅开发/预览）
//!   --glass-test  启动时强制 Acrylic，便于对比材质

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(windows, feature = "gpui-ui"))]
fn main() {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use acaja::config::PresetStore;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    let args: Vec<String> = std::env::args().collect();
    let diag = args.iter().any(|a| a == "--diag");
    let standalone = args.iter().any(|a| a == "--standalone");
    let glass_test = args.iter().any(|a| a == "--glass-test");

    let _log_path = init_logging(diag);
    install_panic_hook();
    log::info!("ACAJA 设置进程启动");

    // 设置进程单例（与旧版同名，避免两个设置窗口同时改同一份配置）
    let mutex: HANDLE =
        unsafe { CreateMutexW(None, false, w!("Local\\ACAJACrosshairSettings")) }
            .unwrap_or_default();
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        log::warn!("设置进程已在运行");
        return;
    }
    let _mutex_guard = mutex;

    // 主进程自检：主进程消失（退出/崩溃）→ 设置进程兜底退出。
    if !standalone {
        std::thread::spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if acaja::ipc::find_backend().is_none() {
                std::process::exit(0);
            }
        });
    }

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

    acaja::ui::run(Arc::new(Mutex::new(store)), glass_test);
    log::info!("设置进程正常退出");
}

#[cfg(not(all(windows, feature = "gpui-ui")))]
fn main() {
    eprintln!("acaja-ui 需要 Windows + `gpui-ui` feature");
}

/// 发行版默认不写日志；`--diag` 或 debug 构建例外。崩溃日志另行恒定写入。
fn init_logging(diag: bool) -> Option<std::path::PathBuf> {
    if !cfg!(debug_assertions) && !diag {
        return None;
    }
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(if diag { "acaja-ui-diag.log" } else { "acaja-ui.log" });
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

/// 与主程序共用 `acaja-crash.log`；发行版 panic 必须可诊断。
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
            let _ = writeln!(file, "[acaja-ui] {info}");
        }
    }));
}
