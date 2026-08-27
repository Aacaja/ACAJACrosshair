//! ACAJA 入口：日志初始化 → 单实例守卫 → 配置迁移 → 覆盖层预览。
//!
//! S2 里程碑版本：配置层 + 覆盖层渲染已就绪；
//! 启动后准星显示在屏幕中央，点确定退出（S3 起替换为托盘/热键常驻）。

// 发布版去掉控制台窗口（GUI 子系统）；debug 构建保留控制台便于调试
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use log::{info, warn};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_OK};
use windows::core::w;

use acaja::config::{migrate_legacy, PresetStore};
use acaja::overlay::OverlayHandle;
use acaja::{APP_NAME, VERSION};

/// 单实例互斥体名称（Local 会话级，无需提权）
const SINGLE_INSTANCE_MUTEX: &str = "Local\\ACAJACrosshair_SingleInstance";

/// 初始化文件日志：`%APPDATA%/ACAJACrosshair/acaja.log`（追加模式）
fn init_logging() -> Option<PathBuf> {
    let dir = acaja::appdata_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    let log_path = dir.join("acaja.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok()?;
    simplelog::WriteLogger::init(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        file,
    )
    .ok()?;
    info!("日志文件：{}", log_path.display());
    Some(log_path)
}

/// S2 覆盖层预览
fn run_overlay_preview() {
    let (overlay, thread) = acaja::overlay::start();

    // 默认预设 + 屏幕中心
    let preset = Arc::new(acaja::config::Preset::default());
    let center = acaja::overlay::primary_screen_center();
    info!("显示准星于 {:?}", center);
    overlay.update(preset, center, true);

    let _ = unsafe {
        MessageBoxW(
            None,
            w!("ACAJA S2 预览\n\n准星应显示在屏幕中央（红色十字）。\n\n点击确定退出预览。"),
            w!("ACAJA"),
            MB_OK,
        )
    };

    overlay.close();
    let _ = thread.join();
    info!("预览结束");
}

fn main() {
    init_logging();
    info!("{} v{} 启动", APP_NAME, VERSION);

    // ---- 单实例守卫 ----
    let mutex: HANDLE =
        unsafe { CreateMutexW(None, false, w!("Local\\ACAJACrosshair_SingleInstance")) }
            .unwrap_or_default();
    let already_running = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already_running {
        warn!("检测到已有实例在运行，本实例退出");
        return;
    }
    // 句柄保持到进程退出
    std::mem::forget(mutex);

    // ---- 配置目录 + 旧版迁移 ----
    let appdata = match acaja::appdata_dir() {
        Ok(d) => d,
        Err(e) => {
            warn!("无法获取 APPDATA: {e}");
            return;
        }
    };
    let legacy_dir = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("CrosshairApp");
    match migrate_legacy(&legacy_dir, &appdata) {
        Ok(report) if report.presets > 0 => {
            info!("旧版配置迁移完成：{} 个预设", report.presets);
        }
        Ok(_) => info!("无需迁移旧版配置"),
        Err(e) => warn!("旧版配置迁移失败: {e}"),
    }
    let store = match PresetStore::open(&appdata) {
        Ok(s) => s,
        Err(e) => {
            warn!("配置仓库打开失败: {e}，使用默认配置");
            return;
        }
    };
    info!("当前预设：{}", store.active_name());

    run_overlay_preview();
}