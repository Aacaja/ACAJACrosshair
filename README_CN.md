# ACAJA —— Windows 准星覆盖工具

>[!NOTE]
> **开发中（Milestone S2）**：渲染核心与配置系统已完成并通过 CI 验证；系统集成（托盘/热键）、输入层（手柄 ADS）、设置界面正在按计划推进，见文末 [Roadmap](#roadmap)。

ACAJA 是一个 Windows 桌面准星覆盖工具，用 **Rust** 编写（`windows-rs` + Direct2D），是原 [CrossHairLIN](https://github.com/liuroland55/CrossHairLIN)（小林の准星，Python/PySide6）的全新重写版。

可以在游戏画面上叠加自定义准星，**点击穿透**，不影响游戏操作。适用于没有准星、或想自定义准星样式的游戏（建议配合无边框窗口模式）。

## 相比旧版（Python 版）的核心改进

| 维度 | 旧版 (v3.1.0, PySide6) | ACAJA (Rust) |
|---|---|---|
| 产物大小 | 46 MB 单 exe | **~2.7 MB** 单 exe |
| 架构 | 全屏透明窗口 + 20FPS 定时重绘 | **事件驱动 + 只绘制准星小窗口**，静止时 ≈0% CPU |
| 渲染 | QPainter（CPU） | **Direct2D 硬件加速**（GPU 合成） |
| 启动 | 约 1~2 秒 | 毫秒级 |
| 点击穿透 | `WindowTransparentForInput` | 原生 `WS_EX_LAYERED\|TRANSPARENT` 分层窗口 |
| 内置功能 | 8 种形状 | **14 种形状**（含多色四象限、动态扩散、后坐力指示） |
| 语言 | Python 3.8+ / PySide6 依赖 | 零运行时依赖，双击即用 |

## 已实现功能（S0–S2）

- **14 种准星形状**：十字、圆点、方块、圆圈、空心十字、空心方框、空心十字加点、箭头 (Chevron)、三角、V、T、四角括号、GapHair、自定义图片
- **多色四象限准星**：上下左右四臂独立配色（对齐 Crosshair X 标志性功能）
- **描边 / 旋转 / 透明度 / 粗细 / 缺口 / 中心点** 全参数可调
- **动态准星**：开火扩散 + 按恢复速度衰减（后坐力恢复指示条）
- **自定义图片**：PNG 一次解码 + 缩放缓存，不反复读盘
- **配置系统**：`%APPDATA%/ACAJACrosshair/` 下按预设分文件存储（原子写入，防写坏）
- **旧版配置一键迁移**：首次运行自动把 `%APPDATA%/CrosshairApp/*.json` 迁移为新格式（形状/大小/颜色/位置/热键/描边/手柄设置等全字段映射），旧目录保留不删
- **全局热键解析**：兼容旧版 `Ctrl+F1` 风格字符串（注册系统在 S3 接入）
- **单实例守卫、文件日志**：`%APPDATA%/ACAJACrosshair/acaja.log`

## 下载与使用

无需安装 Python / Rust，直接从 GitHub Actions 下载：

1. 打开 [Actions 页面](https://github.com/Aacaja/ACAJACrosshair/actions) → 最新一次运行 → **Artifacts** → 下载 `ACAJA-windows-x64`
2. 解压，双击 `ACAJA-v1.0.0-x64.exe`

> 也可以右键 **workflow_dispatch** 手动触发构建，或等正式发布后从 [Releases](https://github.com/Aacaja/ACAJACrosshair/releases) 下载。

## 架构

```
src/
├── main.rs        # 入口：日志 → 单实例守卫 → 配置迁移 → 启动子系统
├── lib.rs         # 品牌常量与应用目录
├── config.rs      # 预设 schema、原子 JSON 持久化、旧版迁移、热键解析
├── i18n.rs        # 中英双语（编译期检查的字符串表）
└── overlay/
    ├── mod.rs     # D2D 渲染线程：分层窗口 + UpdateLayeredWindow 提交
    └── shapes.rs  # 14 种形状纯几何生成（四象限分色、扩散、包围盒）
```

**渲染架构**（与旧版本质区别）：

- 独立渲染线程 + 跨线程通道（`crossbeam`）接收控制消息
- 只创建**覆盖准星包围盒的小窗口**（16–1024px），不再全屏
- 静止时线程阻塞在通道上（≈0% CPU）；只有配置/位置/显隐变化才重绘
- 渲染链路：D2D `DCRenderTarget` → 32bpp 预乘 DIB → `UpdateLayeredWindow`（DWM 合成）
- 画刷 / 图片均缓存，动画由渲染线程内部状态机衰减，不占主线程

**线程模型**（S3 起完整形态）：

```
主线程      Win32 消息泵（全局热键 / WinEvent 前台检测 / 托盘回调）
渲染线程    D2D 覆盖层（事件驱动，空闲阻塞）
输入线程    手柄 XInput 轮询（250Hz）→ ADS 自动隐藏状态机
UI 线程     egui 设置窗口（独立消息循环）
```

## 构建（GitHub Actions）

代码推送后自动在 `windows-latest` 上构建，产物自动上传：

```yaml
# .github/workflows/build.yml
on: { push: [main, v* tags], workflow_dispatch }
steps: checkout → rust-toolchain → rust-cache → cargo build --release → cargo test → package → artifact
```

- 打 `v*` 标签自动发布 GitHub Release
- 所有单元测试（当前 26 个）随每次构建在 CI 执行

## Roadmap

- [x] **S0** 脚手架 + CI 全链路（2026-08）
- [x] **S1** 配置层：预设 schema、原子持久化、旧版迁移、i18n
- [x] **S2** 覆盖层：D2D 渲染、14 形状、多色、描边、旋转、动态扩散、自定义图片
- [ ] **S3** 系统层：托盘常驻、全局热键、前台游戏检测自动切预设、多显示器、窗口吸附、开机自启
- [ ] **S4** 输入层：Raw Input 开火检测 + **手柄 ADS 自动隐藏**（Apex 场景：左扳机按住隐藏/切换/反向多模式，阈值可调）+ 右键快捷切换
- [ ] **S5** UI：egui 双语设置窗（预览画布、参数面板、游戏绑定表、预设管理、导入导出）
- [ ] **S6** 打磨：图标、README 完整化、首版 v1.0.0 发布

## 配置目录

```
%APPDATA%/ACAJACrosshair/
├── app.json              # 应用级设置（语言/主题/自启/游戏绑定）
├── presets/default.json  # 预设（每预设一个文件）
└── acaja.log             # 运行日志（排查问题用）
```

旧版（CrosshairApp）配置**不会**被删除，迁移失败也不影响新程序运行。

## 系统要求

- Windows 10 / 11（x64）
- 无任何运行时依赖

## 常见限制

- **独占全屏（D3D exclusive fullscreen）无法覆盖**：这是硬件级显示独占，任何不动游戏的 overlay 方案都做不到；请使用无边框窗口模式（行业标准做法）。
- 手柄原生支持 Xbox 系（XInput）；DS4/DS5 需 DS4Windows 等驱动模拟。

## 许可

MIT License。

原项目作者：林晓CCC —— B站：https://space.bilibili.com/622769073