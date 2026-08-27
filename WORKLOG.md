# ACAJA 开发日志（Worklog）

> 本文件是项目开发进度的事实记录。新 agent 接入时先读此文件 + `DOCS_DEV_PLAN.md` + `README.md`。

---

## [2026-08-27/28] v1.0.4 + v1.0.5：22 形状+模板 / LB 触发 / preset 同步修复 / 性能优化（内存 CPU）/ 品牌 A 图标

总目标：① 更多形状（借鉴 Crosshair X）② 手柄 LB 触发 ③ 修复"松手后样式变十字+位置偏移"bug ④ 大幅降低常驻内存与 CPU ⑤ 更换品牌 A 图标。

状态：✅ 完成（v1.0.5 CI success，已发 tag + Release）

干到哪了：
- **v1.0.4**：Shape 枚举 14→22（新增 CrossDot/XShape/RingDot/DoubleRing/CircleCross/Gate/ChevronDown/CornerDots，全几何+预览+翻译）；8 个风格模板（Apex 四段+点/Valorant 门形/CS2 绿十字/狙击圆环/经典红/厚门形/纯点/X 形）；AdsButton 扩展 LeftBumper/RightBumper（XInput wButtons 0x0004/0x0008）；**根因修复**：UI 与消息线程各持 preset 副本导致手柄事件回跳旧样式/位置 → 引入 `state::SharedPreset`（version+Arc<Preset>，RwLock 共享），UI push_to_overlay 时版本号+1，消息线程 sync_from_ui 检测版本重注册热键，所有事件处理改从共享读取。
- **v1.0.5（性能）**：设置窗口改为**按需创建**——点 X = 真正关闭并释放 egui/GL 全部资源（此前隐藏常驻 ≈90MB+）；托盘「打开设置」→ reopen 标志 → 主线程 `'ui_loop` 重新 run_native；UI 关闭后消息线程节拍 10ms→50ms；overlay 渲染线程空闲**纯阻塞**（0 唤醒）。预期常驻内存 ~10-20MB、CPU ≈0-0.2%。
- **品牌 A 图标**：PIL 生成（渐变蓝紫圆角底 + 白粗体 A + 横杠红色准星 + 右下白点）→ `assets/icons/ACAJA.ico`（7 尺寸手工 ICO 容器）+ 512/64 png；build.rs winres → 新 ico；托盘加载顺序 = exe 资源(MAKEINTRESOURCE 1) → 文件 → **自绘像素字母 A**（点线距离算法，白 A 透明底）；egui 窗口图标 = include_bytes 64px PNG → IconData。A.lnk 误提交已移除 + .gitignore 加 *.lnk。
- 验证证据：v1.0.4/v1.0.5 CI 全绿（45+ 单测）；踩坑：LoadImageW hinst 传值不能 Some、tray.rs 无 #[cfg(test)] 锚点。

下一步：用户下载 v1.0.5 实测内存/CPU 与图标；若满意可正式发布 Release（tag 已推）。

---
## [2026-08-27] v1.0.3：设置窗口崩溃的真凶——微软雅黑与 ab_glyph 不兼容，内置字体子集根治

总目标：用户报告 UI 起不来（v1.0.1 秒退、v1.0.2 “设置窗口内部错误已捕获”）。日志定位：`epaint fonts.rs:210 PANIC: Error parsing "msyh" TTF/OTF font file: InvalidFont`。同时交付「关闭=隐藏、托盘可重开设置」的 UI 常驻能力。

状态：✅ 完成（CI success；v1.0.3 release 已发，3 资产）

干到哪了：
- **根因锁定**：运行时装字体（msyh.ttc 提取）送给 egui `set_fonts` 时 epaint 内部 panic。CI 的 `real_system_fonts_parse` 在 windows runner 同样复现 = 非用户机器特例，是 **微软雅黑与 ab_glyph 0.2.11 的解析兼容问题**（社区已知坑）。
- **根治方案**：fonttools 子集化成 **内置字体** `assets/fonts/ACAJACJK-Regular.otf`（Noto Sans SC → GB2312 全 6763 汉字 + 标点 + ASCII，1.57MB）→ `include_bytes!` 内嵌；加载前 ab_glyph 实测验证（同版本 0.2.11 加 Cargo 依赖）；系统字体降级为后备并逐候选验证。
- **UI 常驻能力**（用户“找不到 UI 开关”）：点窗口 X = `CancelClose` + `Visible(false)` 隐藏到托盘（不是关闭）；托盘菜单「打开设置」=`show_settings_window()`（Visible(true)+Focus）；托盘「退出」= `QUIT_REQUESTED` + Close。
- 验证证据：commit 095f53d → CI success（`embedded_font_parses` 通过，42+ 单测）；v1.0.3 release 3 资产（exe ~9.4MB）。
- 本机生成物：`/tmp/fontenv`（venv+fonttools）+ `/tmp/NotoSansCJKsc-Regular.otf`（16MB 源）+ `/tmp/cjk_chars.txt`（7551 字符）+ subsets 脚本。

下一步：用户下载 v1.0.3 实测。预期：设置窗口正常出现（内置字体），关闭窗口=隐藏，托盘可随时重新打开设置。

---

## [2026-08-27] v1.0.2：秒退修复（单实例残留提示 + UI 失败常驻）

总目标：用户报告 v1.0.1“打开秒退”。根治两处静默退出路径，任何情况下程序都能自解释。

状态：✅ 完成（CI success，tag v1.0.2 release 已发：exe 3 资产齐全）

干到哪了：
- 根因候选 1（最可能）：旧版进程（v1.0.0/v1.0.1 托盘常驻）未退出 → 新实例命中单实例互斥体 → **静默 return 退出** = 秒退。修复：命中时 MessageBox 弹窗「ACAJA 已在后台运行」提示。
- 根因候选 2：主线程 eframe::run_native 失败 → Err 分支在旧代码也会继续走收尾退出。修复：`catch_unwind` 包裹 + UI 失败/panic 时弹窗显示错误文本 + **进入无 UI 常驻模式**（准星+托盘继续可用，托盘退出经 quit flag 结束进程）。
- 消息线程 CMD_QUIT/WM_QUIT：UI 未开时改为置 `quit` AtomicBool（主线程常驻循环轮询），UI 开着时仍走 ViewportCommand::Close。
- 验证证据：commit e726d13 → CI success（42 单测）；v1.0.2 release 3 资产。

下一步：用户下载 v1.0.2 实测。无论结果如何，现在都有弹窗或日志可诊断（`%APPDATA%/ACAJACrosshair/acaja.log` 含 PANIC 行）。

---

## [2026-08-27] v1.0.1：修复设置窗口不显示（egui 必须主线程）

总目标：v1.0.0 用户实测发现只有准星、设置窗口不出现。重构线程模型使 egui 窗口在主线程创建，并增加 panic 日志钩子与 UI 失败兜底（准星+托盘继续可用）。

状态：✅ 完成

干到哪了：
- 根因判断：egui/eframe(OpenGL glow 后端) 在后台线程创建窗口在 Windows 上会静默失败/panic；GUI 子系统无控制台 → 无任何输出，主线程仍显示准星（与用户现象吻合：只见准星无 UI）。
- 重构：`src/main.rs` — 主线程直接调 `acaja::ui::run`（egui）；Win32 消息泵（托盘/热键/WinEvent/RawInput/手柄事件）整体迁移到独立「消息线程」`msg_thread_main`，用 `Arc<AtomicBool> stop` 控制退出；托盘/热键/WM_QUIT 通过 `UI_CTX`（ui/mod.rs 的 OnceLock<egui::Context>）发 ViewportCommand::Close 关闭设置窗口。
- 新增全局 panic hook 写日志（GUI 无 stderr，panic 必须落盘）。
- UI 窗口创建失败不再拖垮程序：warn 后准星+托盘继续运行。
- 版本 1.0.0 → 1.0.1；README 增加完整使用教程（下载/调整/手柄/游戏绑定/热键/托盘/FAQ）。

验证证据：commit 478162e → CI success（42 单测）；tag v1.0.1 → Release 已发布（exe 7.8MB + README + README_CN）。注意：tag 触发时 softprops/action-gh-release 上传 README_CN 时报错（HTML 响应），release 停在 draft，已用 `gh release edit --draft=false` + `gh release upload --clobber` 手动补发完成。

下一步：用户下载 v1.0.1 实测设置界面出现。若 UI 仍不显示 → 看 `%APPDATA%/ACAJACrosshair/acaja.log` 的 PANIC 行（panic hook 已就位），据日志定位。

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