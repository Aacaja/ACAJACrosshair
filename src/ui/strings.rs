//! UI 文案表（key → 中/英）。
//!
//! **单一数据源**：一行 = 一个 key，中文与英文同排。
//! 历史问题（v1.1.6）：旧版是两个独立 match 表（`zh()` / `en()`）加 `_ => ""` 兜底，
//! 新增 key 时只补一边就会在界面上渲染成**空白项**——左侧导航的「手柄 / 自定义图片」
//! 两项就是这么消失的。现在 key 在表格里只出现一次，漏翻译在结构上不可能发生，
//! 未知 key 由单测拦截（见文件末尾 tests）。
//!
//! 表必须按 key 升序（`binary_search` 的前提，由 `table_sorted_and_unique` 强制）。

use crate::config::AdsMode;
use crate::i18n::Lang;

/// 全部 UI 文案：(key, 中文, English)，按 key 升序
const TABLE: &[(&str, &str, &str)] = &[
    ("ads_button", "触发键", "Trigger"),
    ("ads_hold_hide", "按住隐藏", "Hold to hide"),
    ("ads_hold_show", "按住显示", "Hold to show"),
    ("ads_left_bumper", "左肩键 (LB/L1)", "Left Bumper (LB/L1)"),
    ("ads_left_trigger", "左扳机 (LT/L2)", "Left Trigger (LT/L2)"),
    ("ads_mode", "ADS 模式", "ADS mode"),
    ("ads_off", "关闭", "Off"),
    ("ads_right_bumper", "右肩键 (RB/R1)", "Right Bumper (RB/R1)"),
    ("ads_right_trigger", "右扳机 (RT/R2)", "Right Trigger (RT/R2)"),
    ("ads_toggle", "扣动切换", "Toggle per pull"),
    ("autostart", "开机自启", "Start with Windows"),
    ("autostart_failed", "开机自启设置失败", "Failed to update autostart"),
    ("autostart_note", "写入注册表 Run 键（当前用户）；移动程序位置后重新勾选一次即可修复路径", "Writes the per-user registry Run key; re-toggle after moving the program to fix the path"),
    ("backend_connected", "● 已连接主程序，修改实时生效", "● Connected — changes apply live"),
    ("backend_file_mode", "● 推送模式：完成设置后点「应用」即可生效", "● Push mode: finish settings then hit Apply"),
    ("binding_add", "添加绑定", "Add binding"),
    ("binding_added", "绑定已添加 ✓", "Binding added ✓"),
    ("binding_exe", "进程名（如 r5apex.exe）", "Process name (e.g. r5apex.exe)"),
    ("binding_none", "暂无绑定", "No bindings yet"),
    ("binding_note", "进程名见任务管理器「详细信息」；该进程成为前台窗口时自动套用对应预设", "See Task Manager → Details for the process name; the preset applies when it becomes the foreground window"),
    ("binding_remove", "移除", "Remove"),
    ("binding_removed", "绑定已删除 ✓", "Binding removed ✓"),
    ("bindings", "游戏绑定（前台进程 → 自动切预设）", "Game bindings (foreground app → preset)"),
    ("cannot_delete_default", "默认预设不能删除", "Default preset cannot be deleted"),
    ("center_btn", "屏幕居中", "Center Screen"),
    ("center_dot", "中心点大小", "Center dot size"),
    ("color_bottom", "下", "Bottom"),
    ("color_left", "左", "Left"),
    ("color_right", "右", "Right"),
    ("color_top", "上", "Top"),
    ("create", "创建", "Create"),
    ("created", "已创建 ✓", "Created ✓"),
    ("custom_image", "自定义图片（需选择对应形状）", "Custom Image (select the shape first)"),
    ("delete", "删除预设", "Delete Preset"),
    ("deleted", "已删除 ✓", "Deleted ✓"),
    ("dynamic", "动态准星", "Dynamic Crosshair"),
    ("error", "错误", "Error"),
    ("fire_expand", "开火扩散量（px）", "Fire expand (px)"),
    ("gamepad", "手柄（Apex 瞄准吸附）", "Gamepad (Apex aim snapping)"),
    ("gamepad_fire_expand", "右扳机开火驱动扩散", "Right trigger drives spread"),
    ("gamepad_note", "手柄设置保存后立即生效（XInput，无手柄时自动降频轮询）", "Applies right after saving (XInput; polling slows down when no gamepad is connected)"),
    ("hollow_gap", "中心缺口（空心）", "Gap (hollow)"),
    ("hotkey", "快捷键", "Hotkey"),
    ("hotkey_next", "切换下一预设热键", "Next-preset hotkey"),
    ("hotkey_note", "热键保存后立即注册生效（格式 Ctrl+Shift+F1）；被其它程序占用时不会生效", "Registered as soon as you save (format Ctrl+Shift+F1); ignored if another app already owns it"),
    ("hotkey_toggle", "切换准星热键（如 Ctrl+F1）", "Toggle hotkey (e.g. Ctrl+F1)"),
    ("image_path", "图片路径", "Image path"),
    ("image_scale", "图片缩放", "Image scale"),
    ("main_color", "主颜色", "Main Color"),
    ("monitor", "显示器", "Monitor"),
    ("multicolor", "多色模式（四象限独立配色）", "Multicolor (independent quadrant colors)"),
    ("nav_dynamic", "动态准星", "Dynamic"),
    ("nav_gamepad", "手柄", "Gamepad"),
    ("nav_hotkey", "快捷键", "Hotkeys"),
    ("nav_image", "自定义图片", "Image"),
    ("nav_position", "位置", "Position"),
    ("nav_presets", "预设", "Presets"),
    ("nav_style", "形状样式", "Style"),
    ("nav_system", "系统", "System"),
    ("new_name", "新预设名", "New name"),
    ("new_preset", "创建预设（克隆当前参数）", "New preset (clone current)"),
    ("opacity", "透明度", "Opacity"),
    ("outline", "描边", "Outline"),
    ("outline_color", "描边颜色", "Outline color"),
    ("outline_opacity", "描边透明度", "Outline opacity"),
    ("outline_thickness", "描边厚度", "Outline thickness"),
    ("pos_x", "X", "X"),
    ("pos_y", "Y", "Y"),
    ("position", "位置", "Position"),
    ("preset_current", "当前预设", "Current preset"),
    ("presets", "预设", "Presets"),
    ("preview", "实时预览", "Live Preview"),
    ("push_apply", "应用设置到主程序", "Apply to main program"),
    ("pushed_ok", "已应用（实时推送）✓", "Applied (live push) ✓"),
    ("pushed_via_file", "已应用（文件通道，主程序自动加载）✓", "Applied (file channel, auto-loaded) ✓"),
    ("quit_backend", "退出主程序", "Quit main program"),
    ("quit_backend_sent", "已请求主程序退出", "Quit request sent"),
    ("rc_click", "点击切换", "Click to toggle"),
    ("rc_hold_hide", "按住隐藏", "Hold to hide"),
    ("rc_hold_show", "按住显示", "Hold to show"),
    ("recoil_indicator", "后坐力恢复指示条", "Recoil recovery indicator"),
    ("recover_ms", "恢复速度（ms/px）", "Recovery speed (ms/px)"),
    ("right_click", "右键切换准星", "Right-click toggle"),
    ("right_click_mode", "右键模式", "Right-click mode"),
    ("rotation", "旋转", "Rotation"),
    ("save", "保存预设", "Save Preset"),
    ("saved", "已保存 ✓", "Saved ✓"),
    ("shape", "形状", "Shape"),
    ("shape_brackets", "四角括号", "Brackets"),
    ("shape_chevron", "箭头 (^)", "Chevron (^)"),
    ("shape_chevron_down", "向下箭头", "Chevron Down"),
    ("shape_circle", "圆圈", "Circle"),
    ("shape_circle_cross", "圆环十字", "Circle + Cross"),
    ("shape_corner_dots", "四角圆点", "Corner Dots"),
    ("shape_cross", "十字", "Cross"),
    ("shape_cross_dot", "十字加点", "Cross + Dot"),
    ("shape_custom_image", "自定义图片", "Custom Image"),
    ("shape_dot", "圆点", "Dot"),
    ("shape_double_ring", "双圆环", "Double Ring"),
    ("shape_gap_hair", "GapHair", "GapHair"),
    ("shape_gate", "门形（Valorant）", "Gate (Valorant)"),
    ("shape_hollow_cross", "空心十字", "Hollow Cross"),
    ("shape_hollow_cross_dot", "空心十字加点", "Hollow Cross + Dot"),
    ("shape_hollow_square", "空心方框", "Hollow Square"),
    ("shape_ring_dot", "圆环加点（狙击）", "Ring + Dot (sniper)"),
    ("shape_square", "方块", "Square"),
    ("shape_style", "形状与样式", "Shape & Style"),
    ("shape_t_shape", "T 形", "T Shape"),
    ("shape_triangle", "三角", "Triangle"),
    ("shape_v_shape", "V 形", "V Shape"),
    ("shape_x_shape", "X 形", "X Shape"),
    ("size", "大小", "Size"),
    ("snap_note", "跟随前台游戏窗口移动（窗口化 / 无边框窗口有效）", "Follows the foreground game window (windowed / borderless)"),
    ("snap_to_window", "吸附前台窗口", "Snap to foreground window"),
    ("system", "系统", "System"),
    ("template_applied", "模板已套用 ✓", "Template applied ✓"),
    ("template_apply", "套用", "Apply"),
    ("templates", "风格模板", "Style Templates"),
    ("theme_auto", "自动", "Auto"),
    ("theme_dark", "深色", "Dark"),
    ("theme_light", "浅色", "Light"),
    ("thickness", "粗细", "Thickness"),
    ("title", "ACAJA 准星设置", "ACAJA Crosshair Settings"),
    ("tpl_apex", "Apex 四段+点", "Apex 4-dot + center"),
    ("tpl_classic", "经典红色", "Classic red"),
    ("tpl_cross_x", "X 形准星", "X crosshair"),
    ("tpl_cs2", "CS2 绿十字", "CS2 green cross"),
    ("tpl_dot", "纯圆点", "Plain dot"),
    ("tpl_sniper", "狙击圆环", "Sniper ring"),
    ("tpl_thick_gate", "厚门形", "Thick gate"),
    ("tpl_valorant", "Valorant 门形", "Valorant gate"),
    ("tray_quit", "退出", "Quit"),
    ("tray_settings", "打开设置", "Open settings"),
    ("tray_toggle", "显示/隐藏准星", "Show / hide crosshair"),
    ("trigger_threshold", "触发阈值（0-255）", "Trigger threshold (0-255)"),
];

/// 查表：未知 key 返回空串（单测保证 UI 引用的 key 全部存在）
pub fn t(lang: Lang, key: &str) -> &'static str {
    match TABLE.binary_search_by(|row| row.0.cmp(key)) {
        Ok(i) => match lang {
            Lang::Zh => TABLE[i].1,
            Lang::En => TABLE[i].2,
        },
        Err(_) => "",
    }
}

/// 形状名文案
pub fn shape_name(lang: Lang, shape: crate::config::Shape) -> &'static str {
    let key = match shape {
        crate::config::Shape::Cross => "shape_cross",
        crate::config::Shape::Dot => "shape_dot",
        crate::config::Shape::Square => "shape_square",
        crate::config::Shape::Circle => "shape_circle",
        crate::config::Shape::HollowCross => "shape_hollow_cross",
        crate::config::Shape::HollowSquare => "shape_hollow_square",
        crate::config::Shape::HollowCrossDot => "shape_hollow_cross_dot",
        crate::config::Shape::Chevron => "shape_chevron",
        crate::config::Shape::Triangle => "shape_triangle",
        crate::config::Shape::VShape => "shape_v_shape",
        crate::config::Shape::TShape => "shape_t_shape",
        crate::config::Shape::Brackets => "shape_brackets",
        crate::config::Shape::GapHair => "shape_gap_hair",
        crate::config::Shape::CrossDot => "shape_cross_dot",
        crate::config::Shape::XShape => "shape_x_shape",
        crate::config::Shape::RingDot => "shape_ring_dot",
        crate::config::Shape::DoubleRing => "shape_double_ring",
        crate::config::Shape::CircleCross => "shape_circle_cross",
        crate::config::Shape::Gate => "shape_gate",
        crate::config::Shape::ChevronDown => "shape_chevron_down",
        crate::config::Shape::CornerDots => "shape_corner_dots",
        crate::config::Shape::CustomImage => "shape_custom_image",
    };
    t(lang, key)
}

/// ADS 模式名文案
pub fn ads_mode_name(lang: Lang, mode: AdsMode) -> &'static str {
    let key = match mode {
        AdsMode::Off => "ads_off",
        AdsMode::HoldHide => "ads_hold_hide",
        AdsMode::Toggle => "ads_toggle",
        AdsMode::HoldShow => "ads_hold_show",
    };
    t(lang, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Shape;

    #[test]
    fn table_sorted_and_unique() {
        for w in TABLE.windows(2) {
            assert!(w[0].0 < w[1].0, "文案表未按 key 升序: {} >= {}", w[0].0, w[1].0);
        }
    }

    #[test]
    fn every_row_has_both_languages() {
        for (k, z, e) in TABLE {
            assert!(!z.trim().is_empty(), "{k} 缺中文");
            assert!(!e.trim().is_empty(), "{k} 缺英文");
        }
    }

    /// 新增 key 却忘记写进表格 → 界面空白（v1.1.6 导航 bug 的回归测试）
    #[test]
    fn nav_and_system_keys_exist_in_both_langs() {
        for (_, k) in crate::ui::NAV_ITEMS {
            assert!(!t(Lang::Zh, k).is_empty(), "导航 key 缺中文: {k}");
            assert!(!t(Lang::En, k).is_empty(), "导航 key 缺英文: {k}");
        }
        for k in ["tray_toggle", "tray_settings", "tray_quit"] {
            let ok = t(Lang::Zh, k).is_empty() || t(Lang::En, k).is_empty();
            assert!(!ok, "托盘 key 缺失或漏译: {k}");
        }
    }

    /// 形状 / ADS 名若有 key 拼错，下拉框会渲染为空白
    #[test]
    fn every_shape_and_ads_mode_has_label() {
        for s in Shape::ALL {
            assert!(!shape_name(Lang::Zh, s).is_empty(), "形状缺中文: {s:?}");
            assert!(!shape_name(Lang::En, s).is_empty(), "形状缺英文: {s:?}");
        }
        for m in [AdsMode::Off, AdsMode::HoldHide, AdsMode::Toggle, AdsMode::HoldShow] {
            assert!(!ads_mode_name(Lang::Zh, m).is_empty());
            assert!(!ads_mode_name(Lang::En, m).is_empty());
        }
    }
}
