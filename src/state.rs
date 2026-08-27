//! 运行时状态与纯逻辑状态机（可单元测试）。
//!
//! 所有函数都不触碰 Win32，保证 CI 上可完整单测。

use crate::config::AdsMode;
use std::sync::Arc;

/// 共享的当前生效预设：UI 每次推送覆盖层时版本号 +1；消息线程检测版本变化后重同步热键。
/// （v1.0.4 修复：此前 UI 与消息线程各持一份 preset 副本，手柄/热键事件使用旧副本导致
/// 样式回跳与位置漂移）
pub struct SharedPreset {
    pub version: u64,
    pub preset: Arc<crate::config::Preset>,
}

/// 运行时状态
#[derive(Clone, Copy, Debug, Default)]
pub struct AppState {
    /// 准星当前可见性
    pub visible: bool,
    /// 动态扩散量（px）
    pub expand: f32,
}

/// 应用 ADS（瞄准）事件：手柄扳机状态 → 准星显隐决策。
///
/// | 模式 | 行为 |
/// |---|---|
/// | HoldHide（默认） | 按住瞄准 → 隐藏；松开 → 显示 |
/// | HoldShow | 反向：按住 → 显示；松开 → 隐藏 |
/// | Toggle | 每次扣下（上升沿）切换一次 |
/// | Off | 不介入 |
pub fn apply_ads_event(state: &mut AppState, mode: AdsMode, ads: bool) -> bool {
    let mut prev = state.visible;
    match mode {
        AdsMode::Off => return false,
        AdsMode::HoldHide => {
            state.visible = !ads;
        }
        AdsMode::HoldShow => {
            state.visible = ads;
        }
        AdsMode::Toggle => {
            if ads {
                state.visible = !state.visible;
            }
        }
    }
    let changed = prev != state.visible;
    if changed {
        log::debug!("ADS 状态机: mode={mode:?} ads={ads} → visible={}", state.visible);
    }
    changed
}

/// 循环切换到下一个预设（跳过重复；names 为空时返回原值）
pub fn next_preset(current: &str, names: &[String]) -> String {
    if names.is_empty() {
        return current.to_string();
    }
    let Some(pos) = names.iter().position(|n| n == current) else {
        return names[0].clone();
    };
    names[(pos + 1) % names.len()].clone()
}

/// 解析预设位置 → 屏幕坐标（v1.0.6：UI 与消息线程统一从这里取位置）。
/// 中心语义 = 目标显示器「几何中心」（不是工作区中心，避免任务栏导致偏上）。
pub fn resolve_position(p: &crate::config::Preset) -> (i32, i32) {
    let center = crate::system::monitor::by_index(p.position.monitor)
        .map(|m| m.rect_center())
        .unwrap_or_else(crate::overlay::primary_screen_center);
    match (p.position.x, p.position.y) {
        (crate::config::PosVal::Px(x), crate::config::PosVal::Px(y)) => (x as i32, y as i32),
        (crate::config::PosVal::Px(x), _) => (x as i32, center.1),
        (_, crate::config::PosVal::Px(y)) => (center.0, y as i32),
        _ => center,
    }
}

/// 根据前台窗口矩形计算吸附位置（准星位于窗口工作区中心附近，水平居中、垂直略偏上）。
pub fn snap_position(rect: (i32, i32, i32, i32)) -> (i32, i32) {
    let w = rect.2 - rect.0;
    let h = rect.3 - rect.1;
    (rect.0 + w / 2, rect.1 + h / 3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdsMode::*;

    #[test]
    fn ads_hold_hide() {
        let mut s = AppState { visible: true, ..Default::default() };
        assert!(apply_ads_event(&mut s, HoldHide, true));
        assert!(!s.visible);
        assert!(apply_ads_event(&mut s, HoldHide, false));
        assert!(s.visible);
        // 无变化不触发
        assert!(!apply_ads_event(&mut s, HoldHide, false));
    }

    #[test]
    fn ads_hold_show() {
        let mut s = AppState { visible: false, ..Default::default() };
        assert!(apply_ads_event(&mut s, HoldShow, true));
        assert!(s.visible);
        assert!(apply_ads_event(&mut s, HoldShow, false));
        assert!(!s.visible);
    }

    #[test]
    fn ads_toggle_rising_edge() {
        let mut s = AppState { visible: true, ..Default::default() };
        assert!(apply_ads_event(&mut s, Toggle, true));
        assert!(!s.visible);
        // Toggle 模式：松开不改变；再次扣下翻转回 true
        assert!(!apply_ads_event(&mut s, Toggle, false));
        assert!(!s.visible);
        assert!(apply_ads_event(&mut s, Toggle, true));
        assert!(s.visible);
    }

    #[test]
    fn ads_off_noop() {
        let mut s = AppState { visible: true, ..Default::default() };
        assert!(!apply_ads_event(&mut s, Off, true));
        assert!(s.visible);
    }

    #[test]
    fn next_preset_cycles() {
        let names = vec!["default".to_string(), "apex".to_string(), "cs2".to_string()];
        assert_eq!(next_preset("default", &names), "apex");
        assert_eq!(next_preset("apex", &names), "cs2");
        assert_eq!(next_preset("cs2", &names), "default");
        // 未知名 → 第一个
        assert_eq!(next_preset("nope", &names), "default");
        // 空列表
        assert_eq!(next_preset("a", &[]), "a");
    }

    #[test]
    fn snap_position_math() {
        assert_eq!(snap_position((0, 0, 1920, 1080)), (960, 360));
        assert_eq!(snap_position((100, 200, 700, 800)), (400, 400));
    }

    #[test]
    fn resolve_position_works() {
        use crate::config::{PosVal, Position, Preset};
        let mut p = Preset::default();
        p.position = Position { x: PosVal::Px(100.0), y: PosVal::Px(200.0), monitor: -1 };
        assert_eq!(resolve_position(&p), (100, 200));
        // 单轴 Px + Center
        p.position = Position { x: PosVal::Px(50.0), y: PosVal::Center, monitor: -1 };
        let (x, y) = resolve_position(&p);
        assert_eq!(x, 50);
        assert!(y > 0);
        // 全 Center → 必须落在屏幕范围内（CI 有真实显示器）
        let p2 = Preset::default();
        let (cx, cy) = resolve_position(&p2);
        assert!(cx > 0 && cy > 0, "中心解析异常: {cx},{cy}");
    }
}