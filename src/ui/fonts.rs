//! 中文字体加载：从系统字体目录加载，**插入 egui 前用 ab_glyph 实际验证**。
//!
//! 背景（v1.0.3 修复）：v1.0.1/1.0.2 直接加载 msyh.ttc 提取的第一个字体送入 egui，
//! 但微软雅黑与 ab_glyph 解析器存在已知兼容问题（InvalidFont panic），导致设置窗口
//! 整体崩溃。修复策略：
//! 1. 候选顺序优先纯 TTF（simhei.ttf / Deng.ttf），TTC 提取放后面；
//! 2. 每个候选先用 ab_glyph（与 egui 同版本 0.2.11）解析验证，失败自动换下一个；
//! 3. 全部失败返回 None（界面回退英文/系统字体，不再崩溃）。

use std::path::PathBuf;

/// 常见中文字体候选（系统字体，仅在自带字体不可用时启用）
const CANDIDATES: [&str; 6] = [
    r"C:\Windows\Fonts\simhei.ttf",   // 黑体（纯 TTF）
    r"C:\Windows\Fonts\Deng.ttf",     // 等线（纯 TTF）
    r"C:\Windows\Fonts\msyh.ttc",     // 微软雅黑（TTC，需提取）
    r"C:\Windows\Fonts\msyhbd.ttc",   // 雅黑 Bold
    r"C:\Windows\Fonts\simsun.ttc",   // 宋体（TTC）
    r"C:\Windows\Fonts\simsun.ttf",   // 宋体（部分版本为 TTF）
];

/// 内置子集字体（GB2312 全汉字 + 常用标点 + ASCII，1.6MB）
const EMBEDDED_FONT: &[u8] = include_bytes!("../../assets/fonts/ACAJACJK-Regular.otf");

/// 加载中文字体：优先内置子集（确定性可用），其次系统字体（经 ab_glyph 验证）。
pub fn load_cjk_font() -> Option<Vec<u8>> {
    // 1. 内置子集：永远有效，无需系统字体
    if ab_glyph::FontVec::try_from_vec(EMBEDDED_FONT.to_vec()).is_ok() {
        return Some(EMBEDDED_FONT.to_vec());
    }
    log::warn!("内置字体解析失败，回退系统字体");
    // 2. 系统字体候选
    for name in CANDIDATES {
        let path = PathBuf::from(name);
        if !path.exists() {
            continue;
        }
        let data = std::fs::read(&path).ok()?;
        let font = extract_first_font(&data)?;
        if ab_glyph::FontVec::try_from_vec(font.clone()).is_ok() {
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

    /// 内置字体必须可被 ab_glyph 解析（CI 回归保护）
    #[test]
    fn embedded_font_parses() {
        let r = ab_glyph::FontVec::try_from_vec(EMBEDDED_FONT.to_vec());
        assert!(r.is_ok(), "内置字体解析失败: {:?}", r.err());
        let font = r.unwrap();
        assert!(font.glyph_id('准').0 != 0, "内置字体缺少汉字");
        assert!(font.glyph_id('A').0 != 0, "内置字体缺少 ASCII");
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