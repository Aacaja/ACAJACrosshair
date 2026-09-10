//! 原生文件对话框（rfd）：自定义图片选择、预设导入 / 导出。
//!
//! 放在 system 层的原因：对话框是平台能力，UI 只调用这几个返回 `Option<PathBuf>` 的函数，
//! 不必关心 COM/模态细节。对话框是**同步阻塞**的（系统模态行为），期间 egui 不重绘属正常。

use std::path::PathBuf;

/// 选择准星图片（导入自定义准星）
pub fn pick_image() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("选择准星图片")
        .add_filter("图片", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
        .pick_file()
}

/// 选择要导入的预设 JSON
pub fn pick_preset_json() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("导入预设")
        .add_filter("ACAJA 预设", &["json"])
        .pick_file()
}

/// 选择导出目标路径（默认文件名 = 预设名）
pub fn save_preset_json(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("导出预设")
        .set_file_name(format!("{default_name}.json"))
        .add_filter("ACAJA 预设", &["json"])
        .save_file()
}
