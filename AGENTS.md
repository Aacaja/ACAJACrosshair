# AGENTS.md — ACAJA 项目导航（新会话/新 Agent 先读本文件）

> 最后更新：2026-09-11（v1.1.7）
> 配套文档：`WORKLOG.md` = 开发日志（每轮的变更 + **验证证据**，决策脉络都在里面）；
> `README.md` / `README_CN.md` = 用户文档；`DOCS_DEV_PLAN.md` = 早期 S3–S5 并行开发契约（历史资料）。

---

## 0. 一句话

ACAJA = Windows 准星覆盖工具。**Rust**（windows-rs 0.58 + Direct2D + egui 0.30），
双进程：`acaja.exe` 后台壳（准星/托盘/热键/输入，无 UI 框架）+ `acaja-ui.exe` 设置窗（按需拉起，关闭即退出）。

## 1. 架构速查

| 组件 | 文件 | 说明 |
|---|---|---|
| 后台壳进程 | `src/bin/backend.rs` | 单实例互斥体 → 消息窗口 → 托盘/热键/RawInput/前台钩子/手柄 → 事件驱动主循环（`MsgWaitForMultipleObjectsEx`，空闲完全睡眠）|
| 设置进程 | `src/bin/ui.rs` + `src/ui/*` | egui 窗口；每 2s `FindWindow("ACAJABackend")` 自检，主进程没了就退出；关闭窗口 = 本进程退出 |
| 进程间通信 | `src/ipc.rs` | WM_COPYDATA 同步推送（`{"v":visible,"p":<preset json>}`）；UI 另有「写文件 → 主进程 mtime 监控」兜底通道 |
| 覆盖层渲染 | `src/overlay/mod.rs`（D2D 小窗口 + `UpdateLayeredWindow`）、`src/overlay/shapes.rs`（22 形状纯几何，可单测） | 独立渲染线程；空闲阻塞在 channel 上（≈0% CPU）；动态扩散动画在本线程衰减 |
| 配置 | `src/config.rs` | `PresetStore`（每预设一个 JSON，原子写）、`AppConfig`（app.json）、旧版 CrosshairApp 迁移、热键字符串解析 |
| 运行时逻辑 | `src/state.rs` | ADS 状态机、预设轮换、位置解析（纯逻辑，单测覆盖） |
| 输入 | `src/input/gamepad.rs`（XInput 轮询）、`src/input/raw_mouse.rs`（RawInput 全局左右键） | 手柄线程配置 `Arc<RwLock<RuntimeGamepadCfg>>` 动态生效 |
| 系统层 | `src/system/{tray,hotkey,foreground,monitor,autostart}.rs` | 托盘 `Shell_NotifyIcon`、`RegisterHotKey`、`SetWinEventHook` 前台检测、多显示器、注册表自启 |
| 锁工具 | `src/lib.rs` 的 `acaja::sync` | **容忍中毒**的 lock/read/write（见 §4） |
| UI 文案 | `src/ui/strings.rs` | **唯一**文案表（key, 中文, English），导航 key 清单见 `src/ui/mod.rs::NAV_ITEMS` |

数据目录：`%APPDATA%/ACAJACrosshair/`（`app.json`、`presets/<名>.json`、`acaja-crash.log`、`cmd.json`）。

## 2. 环境与验证（**最容易踩的坑**）

开发机是 macOS，**没有 Windows 运行环境**：GUI 外观与 Win32 行为无法本地实测。

1. **主验证通道 = GitHub Actions**（`.github/workflows/build.yml`，windows-latest：`cargo build --release` + `cargo test --release`）。
   `git push origin main` 即触发，约 4 分钟；`gh run list --limit 3` / `gh run watch <id>` 看结果。
   打 tag `vX.Y.Z` 会额外自动发 Release（资产：acaja.exe / acaja-ui.exe / zip / README）。
2. **本地类型校验（v1.1.7 起可用）**——改完先跑它，能省一轮 CI：
   ```sh
   T=~/.rustup/toolchains/stable-aarch64-apple-darwin
   RUSTC=$T/bin/rustc ACAJA_SKIP_WINRES=1 \
     "$T/bin/cargo" check --all-targets --target x86_64-pc-windows-msvc \
     --message-format short 2>&1 | grep ': warning'
   ```
   - 需要该 toolchain 里有 `x86_64-pc-windows-msvc` 的 rust-std（`lib/rustlib/x86_64-pc-windows-msvc`）。
   - `ACAJA_SKIP_WINRES=1` 让 `build.rs` 跳过图标/版本资源嵌入（macOS 上没有 `rc.exe`）。
   - 能查：类型、借用、未使用导入/变量/字段等告警；**不能**查：链接、运行期行为、GUI。
   - ⚠️ **本机 rustc 可能比 CI 的 stable 旧**（本机 1.92.0 / CI 用最新 stable）：**新版本的 lint 只在 CI 出现**。
     实例：`falling back to f32 as the trait bound f32: From<f64> is not satisfied` —— 根因是
     `egui::Stroke::new(1.0, …)` 这种 `impl Into<f32>` 参数收到未标注浮点字面量（将来会变成硬错误），写成
     `1.0_f32` 即可。**「本机零告警」不等于干净，最终以 CI 日志为准。**
   - 解析 `gh run view --log` 的坑：转义符会被写成**字面量 `^[`**（不是 ESC 字节），直接正则匹配 `-->` 会失败，
     先 `text.replace("^[", "\x1b")` 再去 ANSI；筛告警要用 `grep ': warning'`（`--message-format short`
     输出格式是 `文件:行:列: warning: …`，用 `^warning` 会全部漏掉）。
3. 不要提交 `Cargo.lock`（已在 `.gitignore`）。
4. 用户的实机反馈是**最高优先级证据**；没有实机证据时，结论要标注为推断。

## 2.1 发版规则（**每次更新都要做，不要攒**）

每次更新（功能/修复/重构）完成、CI 变绿后，**立即按版本顺序发一个 Release**：

1. 升 `Cargo.toml` 的 `version` —— 严格递增、不跳号、不复用已发过的号（当前 → v1.1.7 → v1.1.8 → …）；
2. 同步 `WORKLOG.md`（写清验证证据）与 README 里的版本号；
3. 提交并推 tag，CI 自动发版：
   ```sh
   git tag vX.Y.Z && git push origin main --tags
   ```
   `.github/workflows/build.yml` 的 `Publish release (tag only)` 步骤会创建 Release，
   资产 = `acaja.exe` / `acaja-ui.exe` / `ACAJA-vX.Y.Z-x64.zip` / `README.md` / `README_CN.md`；
4. 确认结果：`gh release list --limit 3`、
   `gh release view vX.Y.Z --json assets -q '.assets[].name'`（应看到 5 个资产）；
5. **已发布的 tag 不再移动或重打**（会让 Release 与代码不一致）；发错了就补一个更高的版本号。

## 2.2 设计铁律（用户明确要求，改动前先对照）

1. **主程序（`acaja.exe`）绝对干净轻量**：不链接 UI 框架、不加线程/定时轮询、不引入新依赖、
   不加系统级钩子（`WH_KEYBOARD_LL` 之类一律不做）。常驻内存目标 ~15MB、空闲 CPU ≈0%。
   **任何重量级能力只能进设置进程**（`acaja-ui.exe` 用完即走，占用无所谓）。
2. **不做游戏内快速面板/HUD**：用户在游戏里只要准星本身，改参数请开设置窗口。
3. 新功能先问「它在哪个进程里跑、给主程序加了多少成本」；答案不清晰就不要做。
4. 界面可以激进（玻璃材质/动画/大色块），但**功能优先级永远高于视觉**。

## 3. 代码约定

- **文案只有一份表**：`src/ui/strings.rs`（一行 = 一个 key，中英同排，按 key 升序，`binary_search` 查表）。
  新增文案 = 在表里加一行；新增导航项 = 改 `src/ui/mod.rs::NAV_ITEMS`（单测会校验双语均非空）。
  历史上 `zh()`/`en()` 两份独立 match 表 + `_ => ""` 兜底导致导航出现**空白项**，不要再回到那种结构。
- **锁**：一律用 `acaja::sync::{lock, read, write}`，不要写 `.lock().unwrap()`。
  持锁 panic 会让 `Mutex` 永久中毒，`.unwrap()` 会把「一次 panic」放大成「之后每次都 panic」的砖头。
- **不要 panic**：发行版是 `windows_subsystem = "windows"` 且默认不写日志 → panic 等于静默死亡。
  线程创建失败 → 降级 + `warn!`；可失败的调用用 `let _ =` 或 `match` 处理；不要 `expect`。
- **panic 必须可诊断**：panic 钩子恒定写 `%APPDATA%/ACAJACrosshair/acaja-crash.log`（不受 `--diag` 开关影响）。
- **配置字段**：一律 `#[serde(default)]`（旧 JSON 必须能加载）；不要留「从未被读取」的字段。
- 中文注释解释**为什么**（尤其是 Win32 反直觉处），不要复述代码在做什么。
- 缩进 4 空格；文件内保持既有的分节注释风格（`// ===...`）。

## 4. 已验证的坑（改这几处前先看）

- **windows-rs 0.58**：可选句柄传 `None` 而不是 `Some(x)`；`Error::from_win32()` 无参；`EndDraw(None, None)`；
  `DrawBitmap` 5 参；`SetWinEventHook` 返回 `HWINEVENTHOOK` 而非 `Result`；`RegCreateKeyExW` 返回 `WIN32_ERROR`。
- **GDI 位图生命周期**：`DeleteObject` 对**已选入 DC 的位图**会失败（静默泄漏）。销毁前先
  `SelectObject(hdc, 原stock位图)` 换出再删（`src/overlay/mod.rs` 的 `create_dib`/`ensure_canvas_size`）。
- **托盘菜单**：`TrackPopupMenu` 前必须 `SetForegroundWindow`，之后 `PostMessage(WM_NULL)`；
  否则菜单可能收不到输入、点别处不消失（用户看到「卡住」）。且不要在主消息线程弹 `MessageBox`（会卡住托盘/热键）。
- **egui 0.30**：`painter.hline(x: Rangef, y: f32, stroke)`；`rect.center()` 是 `Pos2`（取 `.y`）；
  `ComboBox::from_id_salt`。CJK 字体走内置子集 `assets/fonts/ACAJACJK-Regular.otf`（系统 msyh 与 ab_glyph 不兼容，会 panic）。
- **XInput**：链接名必须是 `XInput`（SDK 库名）；`XINPUT_GAMEPAD` 16 字节；LB/RB 是 `0x0100/0x0200`（`0x0004/0x0008` 是十字键）。
- **RawInput**：`RIDEV_INPUTSINK` 才能后台收到；两阶段 `GetRawInputData`；`WM_INPUT` 必须在消息泵里分支消费。
- **前台检测**：钩子回调由**安装线程**的消息泵驱动，回调里只做轻量工作（开进程查路径即可，别做重活）。
- **开机自启**：注册表 Run 键必须指向**主程序**。设置进程用
  `system::foreground::window_process_path(find_backend())` 反查主程序真实路径（改名/移动也不写错）。
- **位置语义**：中心 = 目标显示器**几何中心**（不是工作区中心，否则任务栏会让准星偏上）；UI 与后台必须共用 `state::resolve_position`。

## 5. 常见操作

```sh
# 本地类型校验（见 §2）
# CI 状态
gh run list --limit 3
gh run watch $(gh run list --limit 1 --json databaseId -q '.[0].databaseId')
# 发版（会触发 Release）
# git tag v1.1.7 && git push origin v1.1.7
```

用户端排查：主程序加 `--diag` 参数运行会写 `%APPDATA%/ACAJACrosshair/acaja-diag.log`；
崩溃信息恒定写在 `acaja-crash.log`；配置文件可手改（`app.json` / `presets/*.json`）。

## 6. 未来分支路线：WebView2 网页界面（2026-09-11 评估，**暂不实施**）

用户问过能否把设置界面改成网页。结论：**可行，但不是银弹**，留作以后可选分支。要点（评估过、留档）：

**关键事实（先破误区）**：CSS `backdrop-filter` **无法采样桌面**（Chromium 视透明窗口背后为空像素，Microsoft/Chromium issue 有记录），
所以「换网页 = 自动有桌面毛玻璃」是错的 —— 桌面磨砂**只能由系统合成器提供**，也就是我们已经实现的
`system/windowfx.rs`（亚克力 / Win11 背板 / Aero）那套，**可被网页宿主直接复用**。

**推荐的实施形态**（若将来要做）：
1. **设置进程内嵌 WebView2**（首选）：宿主窗口加系统模糊 + WebView2 背景透明（`put_DefaultBackgroundColor(透明)` /
   `WEBVIEW2_DEFAULT_BACKGROUND_COLOR=00FFFFFF`，须在初始化前设置），页面透明处即显示系统模糊后的桌面；
   页面内部再用 CSS `backdrop-filter` 做页内层级玻璃。
2. 备选：本地 HTTP 服务 + 浏览器标签页（无桌面磨砂、无依赖，适合顺便从别的设备调参）。
3. 不推荐 Tauri 重写（多一层框架，收益与 1 相同）。

**收益**：CSS 表现力（页内模糊/混合模式/SVG 滤镜/任意缓动/字体渲染）、**改样式无需重编译 Rust**（秒级迭代）、可用现成组件库。
**代价**：WebView2 运行时依赖（Win11 自带；Win10 通常随 Edge 存在，可在检测缺失时提示或内置固定版 ~180MB）、
设置进程内存 ~100–200MB、界面整体重写（现有 egui 约 3400 行 → HTML/CSS/JS 约 2500–3000 行）、
无边框拖拽/缩放、DPI、字体嵌入都要重做。

**PoC 范围（约 1 轮，不改默认行为）**：给 `acaja-ui.exe` 加 `--web` 开关 →
透明 WebView2 + 宿主系统模糊 + 一个真实页面（读取并实时推送当前预设）+ **原生界面保留为回退**
（WebView2 缺失/初始化失败自动落回 egui）。用户实测对比后再决定是否迁移。
Rust 侧可用 `webview2-com`（微软官方绑定）或 `wry`；仓库已经有 `windows` crate 依赖。

## 7. 当前边界 / 已知未做

- 独占全屏（D3D exclusive）游戏无法覆盖（系统级限制，任何 overlay 工具都不行）。
- 自定义图片目前手填路径（没有文件选择器）；游戏绑定需手填进程名。
- 前台检测只在窗口切换/移动结束时刷新；吸附位置为「水平居中、垂直 1/3 处」。
- 设置界面改动通过 WM_COPYDATA 实时推送 + 文件 mtime 兜底；托盘菜单打开期间（模态循环）推送会被丢弃一次。
