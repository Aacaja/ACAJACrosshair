//! ACAJA —— 准星覆盖工具核心库
//!
//! 双进程架构（v1.1.0 起）：
//! - `acaja.exe`（`src/bin/backend.rs`）= 后台壳：覆盖层 + 托盘 + 热键 + 输入，不链接 UI 框架；
//! - `acaja-ui.exe`（`src/bin/ui.rs` + `ui`）= 设置进程：egui 窗口，关闭即退出；
//!   两者经 `ipc`（WM_COPYDATA）实时同步参数。
//!
//! 模块：
//! - `config`   配置中心（预设、原子持久化、旧版迁移、热键解析）
//! - `i18n`     语言标识（界面文案见 `ui::strings`）
//! - `overlay`  Direct2D 覆盖层渲染（形状几何在 `overlay::shapes`，纯数学、可单测）
//! - `input`    XInput 手柄 / RawInput 鼠标
//! - `system`   前台检测、托盘、全局热键、多显示器、开机自启
//! - `state`    ADS 状态机、预设轮换、位置解析（纯逻辑、可单测）
//! - `ui`       egui 设置界面（文案表、实时预览、字体）

pub mod config;
pub mod i18n;
pub mod input;
pub mod ipc;
pub mod overlay;
pub mod state;
pub mod system;
pub mod ui;

/// 品牌名（英文）
pub const APP_NAME: &str = "ACAJA";
/// 品牌名（中文展示）
pub const APP_NAME_CN: &str = "ACAJA 准星";
/// 版本号，与 Cargo.toml 保持一致
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 应用数据目录：`%APPDATA%/ACAJACrosshair`
///
/// 配置文件、日志、导入的旧版配置备份都在这里。
pub fn appdata_dir() -> std::io::Result<std::path::PathBuf> {
    use std::path::PathBuf;
    let base = std::env::var("APPDATA")
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "APPDATA 未设置"))?;
    Ok(PathBuf::from(base).join("ACAJACrosshair"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_identity() {
        assert_eq!(APP_NAME, "ACAJA");
        assert!(!APP_NAME_CN.is_empty());
    }

    #[test]
    fn version_is_not_empty() {
        assert!(!VERSION.is_empty());
        // 版本号必须为 x.y.z 三段式
        assert_eq!(VERSION.split('.').count(), 3);
    }

}