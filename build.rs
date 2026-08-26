//! 构建脚本：仅在 Windows 目标上嵌入资源（图标 / 版本信息 / DPI manifest）。
//! 非 Windows 环境（如 macOS 开发机上 cargo check）直接跳过，不做任何事。

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        return;
    }

    // ---- 旧版图标沿用（品牌替换时直接换 assets 下文件即可）----
    let mut res = winres::WindowsResource::new();
    res.set_icon("FAV/app.ico");

    // ---- 版本信息 ----
    let version = env!("CARGO_PKG_VERSION");
    let file_version = format!("{version}.0");
    res.set("FileDescription", "ACAJA - Windows crosshair overlay");
    res.set("ProductName", "ACAJA");
    res.set("CompanyName", "ACAJA");
    res.set("LegalCopyright", "MIT License");
    res.set("OriginalFilename", "acaja.exe");
    res.set("FileVersion", &file_version);
    res.set("ProductVersion", &file_version);

    // ---- Per-Monitor DPI manifest + Win10/11 兼容声明 ----
    res.set_manifest(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <!-- Windows 10 / 11 -->
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
</assembly>"#,
    );

    res.compile().expect("winres: embed icon/version/manifest failed");
}