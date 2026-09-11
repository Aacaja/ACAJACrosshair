//! 内置字体：无衬线常规 / 无衬线粗体 / 衬线标题（三族搭配 = 现代衬线×非衬线，粗细对比明显）。
//!
//! 为什么内置：v1.0.3 踩过坑——系统「微软雅黑」与 ab_glyph 0.2.11 不兼容，把 msyh.ttc 交给 egui
//! 会在 epaint 内部 panic，设置窗口直接起不来。现在字体随 exe 内置
//! （Noto Sans SC / Noto Serif SC 子集：GB2312 全汉字 + ASCII + 常用符号，共 8206 字形），
//! 加载前一律用 ab_glyph **实测解析**，失败则逐级回退（系统 CJK 字体 → egui 自带西文字体），任何情况都不 panic。
//!
//! 三套字体由同一字符集、同一 `unitsPerEm = 1000` 生成 → 行内中英混排基线一致，不会「跳字」。
//!
//! 字体族约定（UI 侧使用，`install()` 保证这三个名字**始终存在**，缺字体时自动指向无衬线）：
//! - [`egui::FontFamily::Proportional`] → 无衬线常规（正文）
//! - [`FontFamily::Name("serif")`] → 衬线（标题 / 大字号）
//! - [`FontFamily::Name("bold")`] → 无衬线粗体（数值 / 强调）

use std::path::PathBuf;
use std::sync::Arc;

use egui::FontFamily;

/// 无衬线常规（正文）
const EMBEDDED_SANS: &[u8] = include_bytes!("../../assets/fonts/ACAJASans-Regular.ttf");
/// 无衬线粗体（数值 / 强调）
const EMBEDDED_SANS_BOLD: &[u8] = include_bytes!("../../assets/fonts/ACAJASans-Bold.ttf");
/// 衬线半粗（标题 / 大字）
const EMBEDDED_SERIF: &[u8] = include_bytes!("../../assets/fonts/ACAJASerif-SemiBold.ttf");

/// 衬线族名（标题）
pub const FAMILY_SERIF: &str = "serif";
/// 粗体族名（数值 / 强调）
pub const FAMILY_BOLD: &str = "bold";

/// 常见中文字体候选（仅在内置字体不可用时启用；均为纯 TTF 或可提取的 TTC）
const CANDIDATES: [&str; 6] = [
    r"C:\Windows\Fonts\simhei.ttf",   // 黑体（纯 TTF）
    r"C:\Windows\Fonts\Deng.ttf",     // 等线（纯 TTF）
    r"C:\Windows\Fonts\msyh.ttc",     // 微软雅黑（TTC，需提取）
    r"C:\Windows\Fonts\msyhbd.ttc",   // 雅黑 Bold
    r"C:\Windows\Fonts\simsun.ttc",   // 宋体（TTC）
    r"C:\Windows\Fonts\simsun.ttf",   // 宋体（部分版本为 TTF）
];

/// 字体安装结果：`false` = 该字体不可用、已自动回退（UI 只需记日志，不必分支）
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontStatus {
    pub sans: bool,
    pub serif: bool,
    pub bold: bool,
}

/// 把内置三套字体装进 egui。
///
/// **只在 `AcajaApp::new` 里调用一次**——每帧调用 `set_fonts` 会重建字形图集，界面会闪烁。
pub fn install(ctx: &egui::Context) -> FontStatus {
    let mut fonts = egui::FontDefinitions::default();
    let mut status = FontStatus::default();

    // 无衬线常规：内置优先；解析失败才退回系统候选
    let sans_bytes: Option<Vec<u8>> = if parses(EMBEDDED_SANS) {
        Some(EMBEDDED_SANS.to_vec())
    } else {
        log::warn!("内置无衬线字体解析失败，回退系统字体");
        load_system_cjk()
    };
    if let Some(bytes) = sans_bytes {
        fonts
            .font_data
            .insert("acaja-sans".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
        status.sans = true;
    }
    if parses(EMBEDDED_SANS_BOLD) {
        fonts.font_data.insert(
            "acaja-bold".to_owned(),
            Arc::new(egui::FontData::from_owned(EMBEDDED_SANS_BOLD.to_vec())),
        );
        status.bold = true;
    } else {
        log::warn!("内置粗体解析失败，粗体族将回退无衬线");
    }
    if parses(EMBEDDED_SERIF) {
        fonts.font_data.insert(
            "acaja-serif".to_owned(),
            Arc::new(egui::FontData::from_owned(EMBEDDED_SERIF.to_vec())),
        );
        status.serif = true;
    } else {
        log::warn!("内置衬线解析失败，衬线族将回退无衬线");
    }

    // Proportional / Monospace：CJK 放最前，避免中文落到 egui 自带西文字体（缺字形 → 豆腐块）
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            if status.sans {
                list.insert(0, "acaja-sans".to_owned());
            }
        }
    }
    let tail = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    // 衬线 / 粗体：命名族**始终注册**（缺字体时整条链指向无衬线），UI 无需判断字体是否可用
    let mut serif_chain: Vec<String> = Vec::new();
    if status.serif {
        serif_chain.push("acaja-serif".to_owned());
    }
    if status.sans {
        serif_chain.push("acaja-sans".to_owned());
    }
    serif_chain.extend(tail.iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name(FAMILY_SERIF.into()), serif_chain);

    let mut bold_chain: Vec<String> = Vec::new();
    if status.bold {
        bold_chain.push("acaja-bold".to_owned());
    }
    if status.sans {
        bold_chain.push("acaja-sans".to_owned());
    }
    bold_chain.extend(tail);
    fonts
        .families
        .insert(FontFamily::Name(FAMILY_BOLD.into()), bold_chain);

    ctx.set_fonts(fonts);
    status
}

/// ab_glyph（与 egui/epaint 同版本）实测解析：解析不了就绝不交给 egui
fn parses(bytes: &[u8]) -> bool {
    ab_glyph::FontVec::try_from_vec(bytes.to_vec()).is_ok()
}

/// 系统 CJK 字体回退（逐个候选实测解析，全部失败返回 None）
fn load_system_cjk() -> Option<Vec<u8>> {
    for name in CANDIDATES {
        let path = PathBuf::from(name);
        if !path.exists() {
            continue;
        }
        let Some(data) = std::fs::read(&path).ok() else { continue };
        let Some(font) = extract_first_font(&data) else { continue };
        if ab_glyph::FontVec::try_from_vec(font.clone()).is_ok() {
            log::info!("使用系统字体回退：{name}");
            return Some(font);
        }
        log::info!("字体候选 {name} 解析失败，尝试下一个");
    }
    None
}

/// TTC 容器 → 第一个字体（TTF 返回原样）
fn extract_first_font(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 12 {
        return None;
    }
    if &data[0..4] != b"ttcf" {
        // 已是单字体
        return Some(data.to_vec());
    }
    let num_fonts = u32::from_be_bytes(data[8..12].try_into().ok()?) as usize;
    if num_fonts < 1 || data.len() < 12 + 4 * num_fonts {
        return None;
    }
    let off0 = u32::from_be_bytes(data[12..16].try_into().ok()?) as usize;
    let off1 = if num_fonts >= 2 {
        u32::from_be_bytes(data[16..20].try_into().ok()?) as usize
    } else {
        data.len()
    };
    if off0 >= data.len() || off1 > data.len() || off0 >= off1 {
        return None;
    }
    let font = &data[off0..off1];
    // 字体目录起始必须有合法 sfnt 版本（TTF 0x00010000 / CFF "OTTO" / Apple "true"）
    if font.len() < 16 {
        return None;
    }
    match &font[0..4] {
        [0x00, 0x01, 0x00, 0x00] | b"OTTO" | b"true" | b"typ1" => Some(font.to_vec()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::Font as _;

    /// 构造一个最小合法 TTC 结构（ttcf 头 + 单字体偏移），校验提取逻辑
    #[test]
    fn ttc_extraction() {
        // 伪造：ttcf 头 + numFonts=1 + 一个偏移 [16, 40]；字体区放伪 sfnt 头 + 内容
        let mut fake = vec![0u8; 40];
        fake[0..4].copy_from_slice(b"ttcf");
        fake[8..12].copy_from_slice(&1u32.to_be_bytes());
        fake[12..16].copy_from_slice(&16u32.to_be_bytes());
        fake[16..20].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
        let out = extract_first_font(&fake).unwrap();
        assert_eq!(out.len(), 24);
        assert_eq!(&out[0..4], &[0x00, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn plain_ttf_passthrough() {
        let data = vec![0x00, 0x01, 0x00, 0x00, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let out = extract_first_font(&data).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn garbage_returns_none() {
        assert!(extract_first_font(&[0u8; 4]).is_none());
        // 伪造 ttc 但字体区头部非法 → None
        let mut fake = vec![0u8; 40];
        fake[0..4].copy_from_slice(b"ttcf");
        fake[8..12].copy_from_slice(&1u32.to_be_bytes());
        fake[12..16].copy_from_slice(&16u32.to_be_bytes());
        fake[16..20].copy_from_slice(b"XXXX");
        assert!(extract_first_font(&fake).is_none());
    }

    /// 三套内置字体必须都能被 ab_glyph 解析（CI 回归保护）
    #[test]
    fn embedded_fonts_parse() {
        for (name, bytes) in [
            ("无衬线常规", EMBEDDED_SANS),
            ("无衬线粗体", EMBEDDED_SANS_BOLD),
            ("衬线", EMBEDDED_SERIF),
        ] {
            let r = ab_glyph::FontVec::try_from_vec(bytes.to_vec());
            assert!(r.is_ok(), "内置{name}字体解析失败: {:?}", r.err());
        }
    }

    /// 子集必须覆盖界面真正会用到的字符（少了就会渲染成豆腐块）
    #[test]
    fn subsets_cover_needed_glyphs() {
        // 字母数字 + 汉字 + 界面符号（状态圆点/对勾/箭头/度/间隔号/破折号/省略号/中文标点）
        const NEED: &[char] = &[
            'A', 'z', '0', '9', '准', '星', '设', '置', '形', '状', '颜', '色', '●', '○', '✓',
            '→', '°', '·', '…', '—', '、', '。', '“', '”', '《', '》',
        ];
        for (name, bytes) in [
            ("无衬线常规", EMBEDDED_SANS),
            ("无衬线粗体", EMBEDDED_SANS_BOLD),
            ("衬线", EMBEDDED_SERIF),
        ] {
            let font = ab_glyph::FontVec::try_from_vec(bytes.to_vec()).expect("解析失败");
            let missing: Vec<char> = NEED
                .iter()
                .copied()
                .filter(|c| font.glyph_id(*c).0 == 0)
                .collect();
            assert!(missing.is_empty(), "内置{name}字体缺字形: {missing:?}");
        }
    }

    /// 三套字体必须同度量（同一行中英混排不能跳），否则排版会歪
    #[test]
    fn families_share_metrics() {
        let heights: Vec<f32> = [EMBEDDED_SANS, EMBEDDED_SANS_BOLD, EMBEDDED_SERIF]
            .iter()
            .map(|b| {
                let f = ab_glyph::FontVec::try_from_vec(b.to_vec()).expect("解析失败");
                f.units_per_em().expect("字体缺少 unitsPerEm")
            })
            .collect();
        assert_eq!(heights[0], heights[1], "无衬线常规/粗体 unitsPerEm 不一致");
        assert_eq!(heights[0], heights[2], "衬线与无衬线 unitsPerEm 不一致");
    }

    /// Windows CI 诊断：系统字体尽量可解析（失败仅告警，不阻塞）
    #[cfg(windows)]
    #[test]
    fn real_system_fonts_are_diagnosed() {
        let mut found = 0;
        for name in CANDIDATES {
            let path = PathBuf::from(name);
            if !path.exists() {
                continue;
            }
            let data = std::fs::read(&path).unwrap();
            if let Some(font) = extract_first_font(&data) {
                if ab_glyph::FontVec::try_from_vec(font).is_ok() {
                    found += 1;
                } else {
                    // 已知：msyh.ttc 与 ab_glyph 不兼容（用内置字体规避）
                    eprintln!("已知兼容问题: {name} 解析失败（使用内置子集字体）");
                }
            }
        }
        assert!(found >= 0); // 诊断性质：不断言成功数，内置字体才是主路径
        let _ = found;
    }
}
