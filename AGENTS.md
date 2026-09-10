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
     "$T/bin/cargo" check --all-targets --target x86_64-pc-windows-msvc
   ```
   - 需要该 toolchain 里有 `x86_64-pc-windows-msvc` 的 rust-std（`lib/rustlib/x86_64-pc-windows-msvc`）。
   - `ACAJA_SKIP_WINRES=1` 让 `build.rs` 跳过图标/版本资源嵌入（macOS 上没有 `rc.exe`）。
   - 能查：类型、借用、未使用导入/变量/字段等告警；**不能**查：链接、运行期行为、GUI。
3. 不要提交 `Cargo.lock`（已在 `.gitignore`）。
4. 用户的实机反馈是**最高优先级证据**；没有实机证据时，结论要标注为推断。

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

## 6. 当前边界 / 已知未做

- 独占全屏（D3D exclusive）游戏无法覆盖（系统级限制，任何 overlay 工具都不行）。
- 自定义图片目前手填路径（没有文件选择器）；游戏绑定需手填进程名。
- 前台检测只在窗口切换/移动结束时刷新；吸附位置为「水平居中、垂直 1/3 处」。
- 设置界面改动通过 WM_COPYDATA 实时推送 + 文件 mtime 兜底；托盘菜单打开期间（模态循环）推送会被丢弃一次。
