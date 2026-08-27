# ACAJA 并行开发契约（S3 系统层 + S4 输入层 + S5 UI）

> 所有 subagent 必须遵守本契约。**禁止** `git commit` / `git push` / 删除他人文件。
> 编译在 GitHub Actions 完成（本机无 Rust 工具链），代码必须一次写准。

## 0. 模块所有权（谁写哪些文件）

| 模块 | 文件 | 负责人 |
|---|---|---|
| UI（现代化 egui 设置窗） | `src/ui/mod.rs`（重写完成）、`src/ui/preview.rs`（新建） | **ui-designer** |
| 系统层（托盘/热键/前台检测/显示器/自启） | `src/system/mod.rs`、`src/system/tray.rs`、`src/system/hotkey.rs`、`src/system/foreground.rs`、`src/system/monitor.rs`、`src/system/autostart.rs` | **backend-worker A** |
| 输入层（手柄 XInput + Raw Input 鼠标） | `src/input/mod.rs`、`src/input/gamepad.rs`、`src/input/raw_mouse.rs` | **backend-worker B** |
| 集成（main.rs 装配 + 消息泵 + state） | `src/main.rs`、`src/state.rs` | **backend-worker C** |

**禁止触碰**：`src/config.rs`、`src/i18n.rs`、`src/overlay/*`、`src/ui/strings.rs`、`src/ui/fonts.rs`（已就绪）。
`src/lib.rs` 需要追加 `pub mod system; pub mod input; pub mod ui;` —— 由主控在集成阶段统一加，subagent 不要改 lib.rs（但可以在任务里列出需要暴露的 pub 项）。

## 1. 已有的关键类型（src/config.rs，只读参考）

```rust
pub struct Preset { shape: Shape, size: f32, thickness: f32, opacity: f32, color: String,
  colors: QuadColors, multicolor: bool, rotation: f32, outline: OutlineConfig,
  hollow: HollowConfig, image: ImageConfig, dynamic: DynamicConfig, position: Position,
  snap_to_window: bool, hotkey_toggle: Hotkey, hotkey_next_profile: Hotkey,
  right_click_toggle: bool, right_click_mode: RightClickMode, gamepad: GamepadConfig, auto_topmost: bool }
pub struct GamepadConfig { ads_mode: AdsMode /*Off|HoldHide|Toggle|HoldShow*/,
  ads_button: AdsButton /*LeftTrigger|RightTrigger*/, trigger_threshold: u8, fire_expand: bool }
pub enum Shape { Cross,Dot,Square,Circle,HollowCross,HollowSquare,HollowCrossDot,Chevron,
  Triangle,VShape,TShape,Brackets,GapHair,CustomImage }  // Shape::ALL: [Shape;14]
pub enum PosVal { Center, Px(f32) }
pub struct Position { x: PosVal, y: PosVal, monitor: i32 }  // monitor -1=跟随前台
pub enum RightClickMode { Click, HoldShow, HoldHide }
pub struct Hotkey { modifiers: u32 /*MOD_* 位标志*/, vk: u32 }  // Hotkey::parse("Ctrl+F1")
pub const MOD_CONTROL:u32=2; MOD_ALT:u32=1; MOD_SHIFT:u32=4; MOD_WIN:u32=8;
pub struct PresetStore { ... }  // Arc<Mutex<PresetStore>> 共享
  // 方法: open(dir), get(name)->Option<&Preset>, get_active(), active_name()->String,
  //  preset_names()->Vec<String>, save_preset(&mut,name,&Preset)->io::Result<()>,
  //  delete_preset(&mut,name)->io::Result<bool>, activate(&mut,name)->bool, save_app()
  // 字段: pub app: AppConfig { pub language:String, pub theme:String, pub last_preset:String,
  //        pub game_bindings: Vec<GameBinding{exe:String,preset:String}>, pub autostart:bool,
  //        pub minimize_to_tray:bool }
pub fn migrate_legacy(old,new)->io::Result<MigrationReport>
```

## 2. overlay 已有 API（src/overlay/mod.rs，只读参考）

```rust
pub struct OverlayHandle { /* Clone 可跨线程 */ }
impl OverlayHandle {
  pub fn update(&self, preset: Arc<Preset>, pos:(i32,i32), visible:bool)
  pub fn update_with_expand(&self, preset: Arc<Preset>, pos:(i32,i32), visible:bool, expand:f32)
  pub fn move_to(&self, pos:(i32,i32))
  pub fn close(&self)
}
pub fn start() -> (OverlayHandle, JoinHandle<()>)
pub fn primary_screen_center() -> (i32,i32)
pub(crate) fn slot_color_pub(colors,&QuadColors, main:(f32,f32,f32), slot:u8)->(f32,f32,f32)
pub fn parse_hex(s:&str)->(f32,f32,f32)
// 另：overlay::shapes::rotation_safe_radius(&ShapeGeom)->f32（预览缩放用）
```

## 3. UI 契约（ui-designer 必须遵守）

```rust
// src/ui/mod.rs 必须提供：
pub fn run(store: Arc<Mutex<PresetStore>>, overlay: OverlayHandle,
           title: &'static str) -> eframe::Result<()>
```
- 使用 `eframe = "0.30"` + `egui = "0.30"`（已依赖）。
- `src/ui/strings.rs` 已含全部文案：`strings::t(lang,key)`、`strings::shape_name(lang,shape)`、`strings::ads_mode_name(lang,mode)`。
- `src/ui/fonts.rs` 已含 `fonts::load_cjk_font() -> Option<Vec<u8>>`（微软雅黑 TTC → egui FontData），在 `App::new` 里 `ctx.set_fonts(...)`。
- 现代化设计要点：深色主题为主（`Visuals::dark()` + 强调色 #0a84ff），圆角卡片（`ui.group` + `Frame::group` 自定义 rounding 8.0）、分区标题（`ui.label(RichText::new(t("shape_style")).strong().size(15.0))`）、滑条 + 数值同排、ComboBox 带 `from_id_salt`、预览棋盘格背景 + 居中准星、颜色 swatch（圆角矩形）+ hex 文本输入、状态提示（保存后显示 ✓ 短暂提示）。
- 工作副本模型：`App` 持有 `preset: Preset`（working copy）；任何控件改动把 `dirty=true`；帧末若 dirty → `overlay.update(Arc::new(preset.clone()), pos, visible)`；位置也在同一消息里。
- 语言/主题切换：写 `store.lock().unwrap().app.language/theme` 并 `save_app()`。
- 预设管理：ComboBox 选预设 → 从 store load 到 working copy（并激活 + save_app）；输入新名 + 创建（克隆当前）；保存当前参数到所选预设；删除（default 禁止）。
- 窗口尺寸建议 `[760, 920]`，`ScrollArea::vertical` 整页滚动。
- 热键输入用 `TextEdit`（存 `Hotkey::parse` 回写 `preset.hotkey_toggle`，显示用 `to_string()`）。
- 显示开关按钮（显示/隐藏准星 → `overlay.update(..., visible)`）+ 退出按钮（`std::process::exit(0)` 或向主线程发 WM_QUIT——集成后由集成层处理，UI 先提供按钮置位一个 `quit` 标志，见下）。
- `App` 实现 `eframe::App`；`on_exit` 里 `store.save_app()`。
- 退出协议：`run()` 返回后主线程负责关闭 overlay。UI 提供「退出程序」按钮 → 调用 `std::process::exit(0)`（S3 托盘常驻后改由消息泵协议，本轮允许直接 exit）。

## 4. 系统层契约（backend-worker A 必须实现）

`src/system/mod.rs` 导出以下（全部 `pub`）：

```rust
// monitor.rs —— 多显示器枚举（already feature Win32_Graphics_Gdi）
pub struct MonitorInfo { pub rect: (i32,i32,i32,i32), pub work: (i32,i32,i32,i32), pub primary: bool }
pub fn monitors() -> Vec<MonitorInfo>
pub fn monitor_at(x:i32,y:i32) -> Option<MonitorInfo>
pub fn work_center(m:&MonitorInfo) -> (i32,i32)
pub fn current_dpi(hwnd) -> u32   // GetDpiForWindow (WindowsAndMessaging)

// hotkey.rs —— RegisterHotKey 封装（挂在主线程消息窗口 hwnd 上）
pub struct HotkeyRegistrar;
impl HotkeyRegistrar {
  pub fn register(hwnd: HWND, id: u32, mods: u32, vk: u32) -> bool  // 内部 RegisterHotKeyW(hwnd,id,mods,vk)
  pub fn unregister(hwnd: HWND, id: u32)
}
pub const HOTKEY_ID_TOGGLE: u32 = 1;
pub const HOTKEY_ID_NEXT_PRESET: u32 = 2;
// WM_HOTKEY (0x0312) 消息在主线程消息泵处理：wParam 低位即 id

// tray.rs —— Shell_NotifyIcon 托盘（消息窗口由集成层提供 hwnd）
pub struct Tray { /* 内部持有 NOTIFYICONDATAW 枚举/句柄 */ }
impl Tray {
  pub fn add(hwnd: HWND, u_id: u32, icon_path_ico: &str, tooltip: &str) -> windows::core::Result<Tray>
  pub fn set_tooltip(&mut self, s: &str) -> windows::core::Result<()>
  pub fn remove(&mut self)
}
pub const WM_TRAYICON: u32 = 0x8000 + 64; // WM_APP+64 托盘回调消息
pub const TRAY_ID: u32 = 1;
// 托盘菜单：WM_TRAYICON 收到 lParam = WM_LBUTTONDBLCLK(0x203)/WM_CONTEXTMENU(0x7B) 时
// 用 TrackPopupMenu + CreatePopupMenu/AppendMenuW 弹菜单（菜单 id: 1001 切换/1002 设置/1003 退出）
// 菜单命令经 PostMessage(hwnd, WM_APP+65, wParam=cmd, 0) 回主线程处理。

// foreground.rs —— SetWinEventHook 显式进程检测（对齐 Crosshair X per-game 自动切预设）
pub fn start_fg_watcher(tx: Sender<FgEvent>) -> windows::core::Result<()>
pub enum FgEvent {
  Changed { exe: String, hwnd: HWND },     // 前台进程变化（GetWindowThreadProcessId+QueryFullProcessImageNameW）
  Moved { rect: (i32,i32,i32,i32) },       // 前台窗口移动/缩放（EVENT_SYSTEM_MOVESIZEEND 节流 100ms）
}
// 实现：SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(cb),
//                       0, 0, WINEVENT_OUTOFCONTEXT)
// 注意回调里不能做重活：只 GetWindowThreadProcessId + OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)
// + QueryFullProcessImageNameW 取 exe 名，发 channel 就返回。
// 注意：SetWinEventHook 的回调在该线程的消息泵驱动（主线程），OK。
// EVENT_SYSTEM_MOVESIZEEND 同时监听；节流（记录上次时间 100ms 内跳过）。

// autostart.rs —— 开机自启（注册表 Run 键）
pub fn set_autostart(enabled: bool) -> windows::core::Result<()>
// HKCU\Software\Microsoft\Windows\CurrentVersion\Run, 值名 "ACAJACrosshair",
// 值 = 当前 exe 路径（std::env::current_exe()）+ 不存在 key 时创建（RegCreateKeyExW）
```

### windows-rs 0.58 已核实的坑（必须遵守）
- **可选句柄参数传 `None`**（推断为 Option<&T>），**绝不传 `Some(hwnd)`**（Option<HWND> 不实现 Param）。
  例：`CreateWindowExW(..., None, None, None, None)`；`SetWindowPos(hwnd, HWND_TOPMOST, ...)`（HWND_TOPMOST 直接传）。
- `Error::from_win32()` **无参数**；自定义错误码：`windows::core::Error::from(WIN32_ERROR)` 或 `Error::from(ERROR_XXX)`（From<WIN32_ERROR> 已实现）。
- `GetSystemMetrics(SM_CXSCREEN)` 在 `Win32::UI::WindowsAndMessaging`（**不在** SystemInformation）。
- `WNDPROC` 是 `type WNDPROC = Option<unsafe extern "system" fn(HWND,u32,WPARAM,LPARAM)->LRESULT>`；WNDCLASSW 字段 `lpfnWndProc: Some(proc)`。
- 句柄类型字段公开：`HWND(pub *mut c_void)`，可 `HWND(std::ptr::null_mut())` 直接构造；也实现 Default。
- `HMODULE`/`HINSTANCE` 字段 `.0` 可直接互转。
- 所有 `ID2D*` 接口方法 unsafe；`EndDraw(None, None)` 两个参数；`DrawBitmap` 只有 5 参数。
- Cargo.toml 已启用 features：Win32_Foundation, Graphics_Direct2D(_Common), Graphics_Dxgi_Common, Graphics_Gdi, Security, System_Com, System_LibraryLoader, System_Registry, System_SystemInformation, System_Threading, UI_Accessibility, UI_HiDpi, UI_Input_KeyboardAndMouse, UI_Shell, UI_WindowsAndMessaging, Foundation_Numerics。
  **新 API 如需要新 feature：不能改 Cargo.toml**（主控统一加）——任务报告里说明需要的 feature，主控添加。XInput 直接 `#[link(name="xinput1_4")]`，不依赖 crate。

### API 自查方法（写代码前必须核对签名！）
已下载的 windows-rs 0.58 源码就在本机：
```
/tmp/wam.rs      Win32/UI/WindowsAndMessaging（Shell_NotifyIconW、RegisterClassW、TrackPopupMenu 等）
/tmp/gdi.rs      Win32/Graphics/Gdi（MONITORINFO、GetMonitorInfoW、MonitorFromPoint）
/tmp/fnd.rs      Win32/Foundation（句柄、RECT/POINT/SIZE、WIN32_ERROR）
/tmp/threading.rs Win32/System/Threading（OpenProcess、GetWindowThreadProcessId）
/tmp/ll.rs       Win32/System/LibraryLoader
/tmp/sysi.rs     Win32/System/SystemInformation
```
新模块需自查：
```
curl -s https://raw.githubusercontent.com/microsoft/windows-rs/0.58.0/crates/libs/windows/src/Windows/Win32/UI/Accessibility/mod.rs -o /tmp/acc.rs        # SetWinEventHook, WINEVENT_OUTOFCONTEXT
curl -s .../Win32/UI/Input/KeyboardAndMouse/mod.rs -o /tmp/kbm.rs                    # RegisterRawInputDevices, GetRawInputData
curl -s .../Win32/System/Registry/mod.rs -o /tmp/reg.rs                              # RegCreateKeyExW/RegSetValueExW/RegDeleteValueW
grep -n "pub unsafe fn XXX" /tmp/xxx.rs | head      # 核对签名后照抄参数
```
**写任何 Win32 调用前先 grep 出真实签名**，不要凭记忆写。

## 5. 输入层契约（backend-worker B 必须实现）

`src/input/mod.rs` 导出：

```rust
// gamepad.rs —— XInput 轮询（零依赖，直接链接 xinput1_4）
pub enum GameEvent { Ads(bool), Fire(bool) }   // Ads(true)=瞄准开始(建议隐藏) Fire(true)=开火
pub struct GamepadWatcher { pub events: Receiver<GameEvent>, /* handle Drop 停止 */ }
pub fn start_gamepad(threshold: u8) -> GamepadWatcher
// 线程：XInputGetState(0) 每 4ms；ERROR_DEVICE_NOT_CONNECTED(1167) 时降频到 500ms 轮询等待插入
// 左扳机 bLeftTrigger >= threshold → Ads(true)；右扳机 → Fire(true)；状态变化才发事件
// ADS 模式（HoldHide/Toggle/HoldShow/Off）状态机在集成层（state.rs）实现，这里只报警告事件
// 结构体定义（注意内存布局）：
#[repr(C)] struct XINPUT_STATE { dwPacketNumber: u32, Gamepad: XINPUT_GAMEPAD }
#[repr(C)] struct XINPUT_GAMEPAD { wButtons: u16, bLeftTrigger: u8, bRightTrigger: u8,
  sThumbLX: i16, sThumbLY: i16, sThumbRX: i16, sThumbRY: i16, dwPaddingReserved: u32 }
#[link(name = "xinput1_4")]
unsafe extern "system" { fn XInputGetState(dwUserIndex: u32, pState: *mut XINPUT_STATE) -> u32; }
// XINPUT_GAMEPAD_LEFT_TRIGGER=0x0004, XINPUT_GAMEPAD_RIGHT_TRIGGER=0x0008（如需按钮位）

// raw_mouse.rs —— Raw Input 全局鼠标（按钮状态，无需钩子）
pub struct RawMouse { /* Drop 时注销 */ }
pub fn register_raw_mouse(hwnd: HWND) -> windows::core::Result<RawMouse>
// 内部 RegisterRawInputDevices(&RAWINPUTDEVICE{ usUsagePage: 0x01, usUsage: 0x02,
//   dwFlags: RIDEV_INPUTSINK, hwndTarget: hwnd })
// 主线程消息泵收到 WM_INPUT (0x00FF)，调用：
pub fn handle_raw_input(lparam: LPARAM) -> Option<RawMouseEvent>
pub enum RawMouseEvent { LeftDown, LeftUp, RightDown, RightUp }
// GetRawInputData(lparam, RID_INPUT, buf, &size, size_of::<RAWINPUTHEADER>()) 解析 RAWINPUT{header,data.mouse}
// usButtonFlags: RI_MOUSE_LEFT_BUTTON_DOWN/UP(0x0001/0x0002), RI_MOUSE_RIGHT_BUTTON_DOWN/UP(0x0004/0x0008)
```

## 6. 集成契约（backend-worker C 必须实现）

`src/state.rs`：运行时状态机
```rust
pub struct AppState {
  pub visible: bool,
  pub expand: f32,
}
// 辅助函数（纯逻辑，可单测）：
pub fn apply_ads_event(state:&mut AppState, mode: crate::config::AdsMode, ads: bool)
  // HoldHide: ads→visible=false, !ads→true；HoldShow 反向；Toggle: ads 上升沿翻转；Off: 不变
pub fn next_preset(name:&str, names:&[String]) -> String  // 循环下一个（跳过当前）
```

`src/main.rs`（完整装配）：
```
1. 日志、单实例（现成代码保留）
2. 迁移 + PresetStore::open → Arc<Mutex>
3. overlay::start()
4. 创建主线程消息窗口（隐藏窗口类 "ACAJAMain"，收 WM_HOTKEY/WM_INPUT/WM_TRAYICON/WM_QUIT）
5. system::tray::add(hwnd, ...)（图标用 FAV/app.ico 绝对路径）
6. hotkey 注册（preset.hotkey_toggle / hotkey_next_profile → id 1/2；切换预设时重注册）
7. system::foreground::start_fg_watcher(tx)：收到 FgEvent::Changed → 查 store.app.game_bindings
   匹配 exe → activate 预设 + 重注册热键 + overlay.update；FgEvent::Moved →
   preset.snap_to_window 时按窗口矩形更新准星位置（窗口客户区中心，move_to）
8. input::raw_mouse::register_raw_mouse(hwnd)；WM_INPUT → Fire 事件驱动 overlay expand
   （preset.dynamic.fire_expand_px）+ right_click 事件（preset.right_click_toggle 时切换显隐，
   处理 HoldShow/HoldHide 模式）
9. gamepad::start_gamepad(threshold)：事件 → apply_ads_event（按 preset.gamepad.ads_mode）
   → overlay.update visible；Fire 事件同理 expand
10. UI 线程：ui::run(store, overlay, title)
11. 主线程消息泵：GetMessageW loop，分发 WM_HOTKEY/WM_INPUT/WM_TRAYICON/WM_QUIT；
    托盘 1003 退出 → PostQuitMessage；UI 线程结束（join 不到）→ 通过 ui quit flag 或
    UI 按钮 exit(0) 简化（本轮 UI 直接 exit(0) 退出整个进程即可，托盘退出同）
```

## 7. 完成标准与汇报格式
完成后汇报：
- 完成的文件清单与公开 API
- 每个 Win32 调用的**核实来源**（grep 行号）
- 无法核实的 API 列表（主控统一查）
- 需要的 Cargo feature 新增（如有）
- 自测（本次以可编译为唯一硬标准；纯逻辑部分可写 #[cfg(test)] 单测）