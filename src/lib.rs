//! ACAJA —— 准星覆盖工具核心库
//!
//! 分模块规划：
//! - `config`   配置中心（预设、迁移、持久化）
//! - `i18n`     中英文本地化
//! - `overlay`  Direct2D 覆盖层渲染
//! - `input`    热键 / 鼠标 / 手柄输入
//! - `system`   前台检测、托盘、多显示器
//! - `ui`       egui 设置界面
//!
//! 当前为 S0 骨架，仅提供品牌常量与应用目录，供 main 与测试使用。

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