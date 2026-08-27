# ACAJA 开发日志（Worklog）

> 本文件是项目开发进度的事实记录。新 agent 接入时先读此文件 + `DOCS_DEV_PLAN.md` + `README.md`。

---

## [2026-08-27] S3+S4+S5：系统层 / 输入层 / 设置界面 / 集成（v1.0.0 完整版，含托盘、热键、手柄 ADS、现代化 egui UI）

总目标：交付功能完整的 ACAJA v1.0.0——覆盖层 + 配置迁移 + 托盘常驻 + 全局热键 + 前台游戏自动切预设 + 手柄 ADS 自动隐藏 + 现代化双语设置界面。CI 全绿、tag v1.0.0 发布 Release、用户可从 Actions/Releases 直接下载使用。

状态：🚧 进行中

干到哪了：

- **S0 脚手架** ✅ —— `Cargo.toml`(windows 0.58 + eframe 0.30) + `build.rs`(winres 图标/DPI manifest) + `.github/workflows/build.yml` 全链路跑通。
  证据：push 后 CI success；`ACAJA-v1.0.0-x64.exe`(GUI 子系统 230KB)。仓库迁移至 `Aacaja/ACAJACrosshair`（原 CrossHairLIN 无推送权限；gh token 已补 workflow scope，一次性浏览器授权）。
  备注：本机（macOS）无 Rust 工具链，**编译唯一出口 = GitHub Actions CI**；每轮提交 ~3-4min 反馈。
- **S1 配置层** ✅ —— `src/config.rs`：预设 schema(14 形状/四象限色/描边/动态/手柄/热键解析)+ 原子 JSON 写 + 旧版 CrosshairApp 一键迁移；`src/i18n.rs` 双语。13 单测过（后增至 26）。
  证据：`cargo test` 全绿（CI），含 `migrate_legacy_presets` 全字段断言。
- **S2 覆盖层** ✅ —— `src/overlay/mod.rs`(D2D DCRenderTarget→32bpp DIB→UpdateLayeredWindow，事件驱动小窗静止≈0%CPU) + `src/overlay/shapes.rs`(14 形状纯几何/四象限/扩散)。用户实测：双击 exe 屏幕中央出现红色十字准星 ✓（关键渲染链路验证通过）。
  证据：CI success + 用户实机确认准星显示。
- **S2.5 README 重写** ✅ —— README/README_CN 全面替换为 ACAJA 文档（删除旧 PySide6 吹嘘文案 INTRODUCTION.md）。
- **API 签名预核实** ✅ —— 全部关键 Win32 调用已从 windows-rs 0.58 源码 grep 核实，源文件缓存于 `/tmp/{wam,gdi,fnd,threading,ll,sysi,shell,acc,reg,uiinput,kbm,d2d...}.rs`（macOS 本机会话内可用）。
  关键结论（已踩坑修正过）：可选句柄传 `None` 不传 `Some(x)`；`Error::from_win32()` 0 参数；`EndDraw(None,None)`；`DrawBitmap` 5 参；`GetSystemMetrics` 在 WindowsAndMessaging；`WNDPROC = Option<fn>`；`SetWinEventHook` 返回 HWINEVENTHOOK 非 Result；WINEVENT/EVENT_SYSTEM 常量在 WindowsAndMessaging；RawInput 在 `Win32::UI::Input` 根模块（需新增 feature `Win32_UI_Input`）；Shell_NotifyIconW 在 `Win32::UI::Shell`；RI_MOUSE_* 常量在 wam.rs(u32)；`RegCreateKeyExW` 返回 WIN32_ERROR 非 Result。
- **并发 subagent 尝试** ⚠️ —— 创建了 `ui-designer`(gpt-5.6-luna)/`backend-worker`(deepseek-v4-flash) 两个 agent 定义 + `DOCS_DEV_PLAN.md` 四模块并行契约；**4 路并行 subagent 全部被 abort（工具不可用）**。用户授权放弃并行、主控直接编写（用户明确：一切以开发效率为准）。`DOCS_DEV_PLAN.md` 保留为设计文档。

下一步（按序，全部主控直写，每步提交→CI 验证→修错循环）：
1. 写 `src/system/*`（monitor/hotkey/tray/foreground/autostart）
2. 写 `src/input/*`（gamepad XInput + raw_mouse）
3. 写 `src/state.rs`（ADS 状态机 + 预设轮换，含单测）
4. 写 `src/ui/mod.rs` + `src/ui/preview.rs`（egui 现代化设置窗；strings.rs/fonts.rs 已就绪）
5. 装配 `src/main.rs`（消息泵 + 全部子系统）+ `lib.rs` 模块声明 + Cargo.toml 加 `Win32_UI_Input`
6. CI 迭代至全绿 → 文档更新 → tag v1.0.0 发布

边界：不碰 `src/config.rs` / `src/i18n.rs` / `src/overlay/*` / `src/ui/strings.rs` / `src/ui/fonts.rs`（已锁定）；不在本机编译；UI 本轮「退出程序」按钮直接 `exit(0)`（托盘常驻后的优雅退出协议后续再补）。

关联：commit 82d209a(S0)…2200238(S2) / 21dc5cc(GUI fix) / 3459af3(README)；仓库 `Aacaja/ACAJACrosshair`；文档 `DOCS_DEV_PLAN.md`。