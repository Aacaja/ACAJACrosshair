//! 中文字体加载：从系统字体目录加载微软雅黑（msyh.ttc）等。
//!
//! egui 的字体解析器不支持 .ttc 集合文件，这里做轻量 TTC 容器解析，
//! 取出第一个字体（微软雅黑 Regular）交给 egui。仅 20 行，零依赖。

use std::path::PathBuf;

/// 常见中文字体候选（按优先级）
const CANDIDATES: [&str; 5] = [
    r"C:\Windows\Fonts\msyh.ttc",   // 微软雅黑 (TTC)
    r"C:\Windows\Fonts\msyhbd.ttc", // 微软雅黑 Bold
    r"C:\Windows\Fonts\simhei.ttf", // 黑体 (TTF)
    r"C:\Windows\Fonts\Deng.ttf",   // 等线 (TTF)
    r"C:\Windows\Fonts\simsun.ttc", // 宋体 (TTC)
];

/// 加载第一个可用的中文字体字节（失败返回 None，UI 仍可运行）
pub fn load_cjk_font() -> Option<Vec<u8>> {
    for name in CANDIDATES {
        let path = PathBuf::from(name);
        if !path.exists() {
            continue;
        }
        let data = std::fs::read(&path).ok()?;
        let font = extract_first_font(&data)?;
        // 简单校验：至少包含表头
        if font.len() < 16 || &font[0..4] != b"OTTO" && &font[0..4] != b"\x00\x01\x00\x00" {
            continue;
        }
        return Some(font);
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
    Some(data[off0..off1].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个最小合法 TTC 结构（ttcf 头 + 一个偏移），校验提取逻辑
    #[test]
    fn ttc_extraction() {
        // 伪造：ttcf 头 + numFonts=2 + 两个偏移 [16, 40]
        let mut fake = vec![0u8; 40];
        fake[0..4].copy_from_slice(b"ttcf");
        fake[8..12].copy_from_slice(&2u32.to_be_bytes());
        fake[12..16].copy_from_slice(&16u32.to_be_bytes());
        fake[16..20].copy_from_slice(&40u32.to_be_bytes());
        // 字体区放一个伪 sfnt 头
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
    }
}