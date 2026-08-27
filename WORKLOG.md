# ACAJA 开发日志（Worklog）

> 本文件是项目开发进度的事实记录。新 agent 接入时先读此文件 + `DOCS_DEV_PLAN.md` + `README.md`。

---

## [2026-08-27] v1.0.1：修复设置窗口不显示（egui 必须主线程）

总目标：v1.0.0 用户实测发现只有准星、设置窗口不出现。重构线程模型使 egui 窗口在主线程创建，并增加 panic 日志钩子与 UI 失败兜底（准星+托盘继续可用）。

状态：🚧 进行中（提交后待 CI 验证）

干到哪了：
- 根因判断：egui/eframe(OpenGL glow 后端) 在后台线程创建窗口在 Windows 上会静默失败/panic；GUI 子系统无控制台 → 无任何输出，主线程仍显示准星（与用户现象吻合：只见准星无 UI）。
- 重构：`src/main.rs` — 主线程直接调 `acaja::ui::run`（egui）；Win32 消息泵（托盘/热键/WinEvent/RawInput/手柄事件）整体迁移到独立「消息线程」`msg_thread_main`，用 `Arc<AtomicBool> stop` 控制退出；托盘/热键/WM_QUIT 通过 `UI_CTX`（ui/mod.rs 的 OnceLock<egui::Context>）发 ViewportCommand::Close 关闭设置窗口。
- 新增全局 panic hook 写日志（GUI 无 stderr，panic 必须落盘）。
- UI 窗口创建失败不再拖垮程序：warn 后准星+托盘继续运行。
- 版本 1.0.0 → 1.0.1；待提交 → CI → tag v1.0.1。

下一步：验证 CI 全绿 → 更新 README 使用教程 → tag v1.0.1 发布 → 用户实测设置界面。

---

## [2026-08-27] S3+S4+S5：系统层 / 输入层 / 设置界面 / 集成（v1.0.0 完整版，含托盘、热键、手柄 ADS、现代化 egui UI）

总目标：交付功能完整的 ACAJA v1.0.0——覆盖层 + 配置迁移 + 托盘常驻 + 全局热键 + 前台游戏自动切预设 + 手柄 ADS 自动隐藏 + 现代化双语设置界面。CI 全绿、tag v1.0.0 发布 Release、用户可从 Actions/Releases 直接下载使用。

状态：✅ 完成

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
1. ~~写 `src/system/*`（monitor/hotkey/tray/foreground/autostart）~~ ✅
2. ~~写 `src/input/*`（gamepad XInput + raw_mouse）~~ ✅
3. ~~写 `src/state.rs`（ADS 状态机 + 预设轮换，含单测）~~ ✅
4. ~~写 `src/ui/mod.rs` + `src/ui/preview.rs`（egui 现代化设置窗；strings.rs/fonts.rs 已就绪）~~ ✅
5. ~~装配 `src/main.rs`（消息泵 + 全部子系统）+ `lib.rs` 模块声明 + Cargo.toml 加 `Win32_UI_Input`~~ ✅
6. ~~CI 迭代至全绿~~ ✅（42 单测）；桌面文档更新 → tag v1.0.0 发布

### 本轮成果（v1.0.0 完整版）

**已完成（验证证据）**：
- S3 系统层：`src/system/*` —— monitor（EnumDisplayMonitors/GetMonitorInfoW 多显示器）、hotkey（RegisterHotKey 每预设热键）、tray（Shell_NotifyIcon + 右键菜单 1001/1002/1003 + 自绘十字图标回退）、foreground（SetWinEventHook 事件驱动前台检测→按 game_bindings 自动切预设 + MOVESIZEEND 窗口吸附）、autostart（注册表 Run 键）
- S4 输入层：`src/input/*` —— gamepad（XInput 250Hz 轮询，左扳机 ADS/右扳机开火，热插拔降频，`#[link(name="XInput")]` 踩坑记录：SDK 库名是 XInput.lib 不是 xinput1_4.lib）、raw_mouse（RIDEV_INPUTSINK 全局左右键，两阶段 GetRawInputData）
- S5 UI：`src/ui/*` —— egui 现代化设置窗（深色主题+强调色、棋盘格实时预览、滑块+数值、四象限多色编辑、描边/动态/位置/热键/手柄/预设管理、中英切换、CJK 字体从 msyh.ttc 运行时提取）
- 集成：`src/main.rs` 主消息泵（WM_HOTKEY/WM_TRAYICON/WM_CONTEXTMENU/事件轮询节拍 10ms）、`src/state.rs`（ADS 四模式状态机 + 预设轮换 + 吸附位置，8 单测）

**CI 证据**：commit 52d2c82 → `test result: ok. 42 passed`、`ACAJA-v1.0.0-x64.exe`（GUI，**8.2MB** 含 egui）。

**踩坑记录（本轮 8 轮修复，已沉淀进 DOCS_DEV_PLAN §4）**：`w!` 宏只收字面量；`FgEvent` 携带 HWND 导致 channel 非 Send；`HRAWINPUT` 在 UI::Input 不在 Foundation；RawInput 在 `Win32::UI::Input` 根模块（feature Win32_UI_Input）；`CreateIconIndirect`/`ICONINFO` 在 WindowsAndMessaging；`MONITORINFOF_PRIMARY` 在 WindowsAndMessaging；`HICON → Param<HGDIOBJ>` 不存在需手工 `HGDIOBJ(icon.0)`；UI 闭包双重 &mut self 借用（color_row 改自由函数、set 闭包改展开、锁作用域先取结果再 flash）；XINPUT_GAMEPAD 实际 16 字节；Drop 类型不能拆字段。

下一步（收尾）：README 状态更新已做 → 打 tag `v1.0.0` 发布 Release（自动发版）→ 用户下载完整版实测（设置界面/托盘/热键/手柄）。

边界：UI「退出程序」按钮本轮用 `exit(0)`（托盘单开设置窗口的优雅协议留待 v1.1）；`hotkey_next_profile` 已接线（热键id=2 循环预设）；「打开设置」托盘项暂为空动作。