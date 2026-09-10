//! 本地化：语言标识（中/英）。
//!
//! 界面文案本身在 `crate::ui::strings`（单一表格，中英同排）。
//! v1.1.7：删除旧的 `Strings` 结构体 + `ZH`/`EN` 静态表——那两个表在 v1.1.0
//! 双进程重构后已无任何调用点（设置界面全部走 `ui::strings::t`），
//! 只是把同一批文案维护了两份（托盘菜单文案曾因此停留在旧版）。

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
}

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
        assert_eq!(Lang::default(), Lang::Zh);
    }
}
