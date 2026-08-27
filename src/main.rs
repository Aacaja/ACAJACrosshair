//! ACAJA 入口：日志初始化 → 单实例守卫 → 拉起各子系统。
//!
//! S0 骨架：验证编译 / 打包 / 日志 / 单实例链路，
//! 后续步骤在此逐步替换为 overlay / input / system / ui 各模块。

// 发布版去掉控制台窗口（GUI 子系统）；debug 构建保留控制台便于调试
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::OpenOptions;

use log::{info, warn};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK};
use windows::core::w;

/// 单实例互斥体名称（Local 会话级，无需提权）
const SINGLE_INSTANCE_MUTEX: &str = "Local\\ACAJACrosshair_SingleInstance";

/// 初始化文件日志：`%APPDATA%/ACAJACrosshair/acaja.log`（追加模式）
fn init_logging() -> Option<()> {
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let log_path = dir.join("acaja.log");
    let file = OpenOptions::new().create(true).append(true).open(&log_path).ok()?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        file,
    )
    .ok()?;
    info!("日志文件：{}", log_path.display());
    Some(())
}

fn main() {
    init_logging();
    info!("{} v{} 启动", acaja::APP_NAME, acaja::VERSION);

    // ---- 单实例守卫 ----
    // CreateMutexW 在互斥体已存在时仍返回有效句柄，通过 GetLastError 区分
    let mutex: HANDLE =
        unsafe { CreateMutexW(None, false, w!("Local\\ACAJACrosshair_SingleInstance")) }
            .unwrap_or_default();
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        warn!("检测到已有实例在运行，本实例退出");
        return;
    }
    // 句柄保持到进程退出（S3 起由独立模块管理）

    info!("ACAJA v{} 骨架启动成功（S0）", acaja::VERSION);
    let _ = unsafe {
        MessageBoxW(
            None,
            w!("ACAJA v1.0.0 骨架启动成功\n\n单实例守卫与日志链路已就绪。"),
            w!("ACAJA"),
            MB_OK,
        )
    };
}