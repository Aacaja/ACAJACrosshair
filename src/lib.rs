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

/// 锁工具：**容忍中毒**（poisoned）的锁访问。
///
/// `Mutex::lock().unwrap()` 在某个线程持锁 panic 之后会**永久**返回 `Err(PoisonError)`，
/// 于是「一次小 panic」被放大成「之后每一次加锁都 panic」→ 常驻进程彻底变成砖头
/// （v1.1.6 用户反馈「偶尔报错后直接卡掉」的放大器就在这里）。
/// 准星/托盘这类常驻程序宁愿带着上一次的数据继续跑，也不该因为一次 panic 停摆。
pub mod sync {
    use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

    /// `Mutex::lock` 的容忍中毒版本
    pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `RwLock::read` 的容忍中毒版本
    pub fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
        l.read().unwrap_or_else(|e| e.into_inner())
    }

    /// `RwLock::write` 的容忍中毒版本
    pub fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
        l.write().unwrap_or_else(|e| e.into_inner())
    }
}

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

    #[test]
    fn poisoned_lock_is_still_usable() {
        use std::sync::{Arc, Mutex};
        let m = Arc::new(Mutex::new(7u32));
        let m2 = m.clone();
        // 另一个线程持锁 panic → 互斥体中毒
        let joined = std::thread::spawn(move || {
            let _guard = m2.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(joined.is_err());
        assert!(m.lock().is_err(), "互斥体应已中毒");
        // 容忍中毒：仍能读写（否则一次 panic 会让主进程之后每次都 panic）
        assert_eq!(*sync::lock(&m), 7);
        *sync::lock(&m) = 9;
        assert_eq!(*sync::lock(&m), 9);
    }
}