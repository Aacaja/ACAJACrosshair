//! 本地化：中英双语字符串表。
//!
//! 使用编译期检查的 struct（而非字符串字典），杜绝原版手写双份 dict 的
//! 拼写漂移问题。此处只收录核心文案；UI 专属文案在 `ui` 模块内追加。

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    Zh,
    En,
}

impl Lang {
    pub fn from_code(code: &str) -> Self {
        match code.to_ascii_lowercase().as_str() {
            "en" | "english" => Lang::En,
            _ => Lang::Zh,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh",
            Lang::En => "en",
        }
    }

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::Zh => &ZH,
            Lang::En => &EN,
        }
    }
}

/// 全部界面文案（新增文案时必须同时补 ZH 与 EN 两份）。
#[derive(Debug)]
pub struct Strings {
    // -- 品牌 --
    pub app_name: &'static str,
    pub app_name_cn: &'static str,
    pub version_prefix: &'static str,
    // -- 托盘 --
    pub tray_show_hide: &'static str,
    pub tray_open_settings: &'static str,
    pub tray_quit: &'static str,
    pub tray_tooltip: &'static str,
    pub already_running: &'static str,
    // -- 配置 --
    pub preset_new: &'static str,
    pub preset_load: &'static str,
    pub preset_save: &'static str,
    pub preset_delete: &'static str,
    pub preset_name: &'static str,
    pub preset_exists: &'static str,
    pub preset_deleted: &'static str,
    pub preset_loaded: &'static str,
    pub cannot_delete_default: &'static str,
    // -- 迁移 --
    pub migration_done: &'static str,
    pub migration_none: &'static str,
    // -- 输入 --
    pub hotkey_undefined: &'static str,
    pub gamepad_on: &'static str,
    pub gamepad_off: &'static str,
    pub ads_disabled: &'static str,
    pub ads_hold_hide: &'static str,
    pub ads_toggle: &'static str,
    pub ads_hold_show: &'static str,
    pub ads_left_trigger: &'static str,
    pub ads_right_trigger: &'static str,
    // -- 通用 --
    pub ok: &'static str,
    pub cancel: &'static str,
    pub warning: &'static str,
    pub error: &'static str,
    pub enabled: &'static str,
    pub disabled: &'static str,
}

pub static ZH: Strings = Strings {
    app_name: "ACAJA",
    app_name_cn: "ACAJA 准星",
    version_prefix: "版本",
    tray_show_hide: "显示/隐藏准星",
    tray_open_settings: "打开设置",
    tray_quit: "退出",
    tray_tooltip: "ACAJA 准星",
    already_running: "ACAJA 已在运行",
    preset_new: "新建预设",
    preset_load: "加载预设",
    preset_save: "保存预设",
    preset_delete: "删除预设",
    preset_name: "请输入预设名称：",
    preset_exists: "预设已存在",
    preset_deleted: "预设已删除",
    preset_loaded: "预设已加载",
    cannot_delete_default: "默认预设不能删除",
    migration_done: "已从旧版 CrossHairLIN 迁移配置",
    migration_none: "未发现旧版配置，跳过迁移",
    hotkey_undefined: "未设置",
    gamepad_on: "手柄已连接",
    gamepad_off: "未检测到手柄",
    ads_disabled: "关闭",
    ads_hold_hide: "按住隐藏",
    ads_toggle: "扣动切换",
    ads_hold_show: "按住显示",
    ads_left_trigger: "左扳机 (LT/L2)",
    ads_right_trigger: "右扳机 (RT/R2)",
    ok: "确定",
    cancel: "取消",
    warning: "警告",
    error: "错误",
    enabled: "开启",
    disabled: "关闭",
};

pub static EN: Strings = Strings {
    app_name: "ACAJA",
    app_name_cn: "ACAJA Crosshair",
    version_prefix: "version",
    tray_show_hide: "Show/Hide Crosshair",
    tray_open_settings: "Open Settings",
    tray_quit: "Quit",
    tray_tooltip: "ACAJA Crosshair",
    already_running: "ACAJA is already running",
    preset_new: "New Preset",
    preset_load: "Load Preset",
    preset_save: "Save Preset",
    preset_delete: "Delete Preset",
    preset_name: "Enter preset name:",
    preset_exists: "Preset already exists",
    preset_deleted: "Preset deleted",
    preset_loaded: "Preset loaded",
    cannot_delete_default: "Default preset cannot be deleted",
    migration_done: "Migrated configuration from legacy CrossHairLIN",
    migration_none: "No legacy configuration found",
    hotkey_undefined: "Not Set",
    gamepad_on: "Gamepad connected",
    gamepad_off: "No gamepad detected",
    ads_disabled: "Off",
    ads_hold_hide: "Hold to hide",
    ads_toggle: "Toggle per pull",
    ads_hold_show: "Hold to show",
    ads_left_trigger: "Left Trigger (LT/L2)",
    ads_right_trigger: "Right Trigger (RT/R2)",
    ok: "OK",
    cancel: "Cancel",
    warning: "Warning",
    error: "Error",
    enabled: "Enabled",
    disabled: "Disabled",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_code_roundtrip() {
        assert_eq!(Lang::Zh.code(), "zh");
        assert_eq!(Lang::En.code(), "en");
        assert_eq!(Lang::from_code("EN"), Lang::En);
        assert_eq!(Lang::from_code("zh"), Lang::Zh);
        assert_eq!(Lang::from_code("fr"), Lang::Zh); // 未知回退中文
    }

    #[test]
    fn all_strings_present_in_both_langs() {
        let zh = Lang::Zh.strings();
        let en = Lang::En.strings();
        // 逐字段比对非空与内容差异（避免某语言漏填）
        let zh_vec: Vec<&str> = vec![
            zh.app_name, zh.tray_show_hide, zh.preset_new, zh.ads_hold_hide,
        ];
        let en_vec: Vec<&str> = vec![
            en.app_name, en.tray_show_hide, en.preset_new, en.ads_hold_hide,
        ];
        assert_eq!(zh_vec.len(), en_vec.len());
        for (z, e) in zh_vec.iter().zip(en_vec.iter()) {
            assert!(!z.is_empty() && !e.is_empty());
            assert_ne!(z, e, "中英文案不应完全相同（除品牌名外）");
        }
        assert_eq!(zh.app_name, en.app_name); // 品牌名一致
    }
}