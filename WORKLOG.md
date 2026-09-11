# ACAJA 开发日志（Worklog）

> 本文件是项目开发进度的事实记录。新 agent 接入时先读 `AGENTS.md`（项目导航）+ 此文件 + `README.md`。
> **发版规则**（v1.1.7 起）：每次更新完成、CI 变绿后**立即按版本顺序发 Release**（`git tag vX.Y.Z && git push origin --tags`），
> 流程与注意事项见 `AGENTS.md` §2.1。

---

## [2026-09-11] v1.2.3：毛玻璃可调（模式 + 强度）—— 把「只有真机能看到的效果」交给用户定

总目标：用户确认系统为 **Windows 11**；并对网页界面给出结论：**暂时不实装**，仅在咨询可行性，
「如果可行或许可以作为以后开发的一个分支路线」。本轮把毛玻璃做成可调项，让他实测后自选最佳组合。

状态：✅ 完成（CI success：**59 单测**；Release **v1.2.3** 已发，5 资产）

干到哪了：
- **毛玻璃模式**（「系统」分区下拉）：自动 / 亚克力 / Win11 背板 / 关闭 → 写 `app.json` 的 `glass.mode`；
  切换时 `windowfx::set_mode` + `reapply()` 立即生效，状态行同步刷新（亚克力是未公开 API，Win11 背板是官方路径，
  两者在不同机器/驱动上的观感与可用性差异明显 —— 交给用户定）。
- **玻璃强度**滑杆 0.30~1.00（越大越透，步进 0.05，仅跨步进时落盘）：`bg_base` alpha 在 150（0.30，较实）
  与 74（1.00，最透）之间线性插值，其余玻璃层按同一系数缩放；**系统模糊不生效时忽略强度**，仍用不透明底兜底。
- 启动恢复上次选择；背板预烘焙缓存键加入 `(blur_active, level)` → 拖动强度滑杆会重烘焙贴图
  （140ms 去抖 + 松手后预约一次到期重绘，保证最后一档一定落到画面上）。
- 系统层：`windowfx` 增加 `set_mode / mode / current / reapply`，窗口句柄缓存复用；`apply()` 按模式选策略
  （Auto 三级回退 / 只亚克力 / 只背板 / 关闭）；`status_line()` 输出模式与生效方式。
- 配置层：`AppConfig.glass: GlassConfig { mode, level }`，均 `#[serde(default)]`（老 app.json 照常加载），
  `level_clamped()` 防手改越界；新增单测覆盖默认值/夹取/往返（59 单测）。

文档：`AGENTS.md` 新增 §6「未来分支路线：WebView2 网页界面（评估留档）」—— 记录关键事实（CSS `backdrop-filter`
采样不到桌面，桌面磨砂只能靠系统合成器）、三种实施形态对比、收益/代价、PoC 范围（`acaja-ui.exe --web` +
透明 WebView2 + 宿主系统模糊 + 原生界面回退）。README 路线同步为「当前阶段：UI 重度开发」，功能迭代 2–4 标记为已取消。

验证证据：commit cb5c9eb → CI success（`test result: ok. 59 passed; 0 failed`，零告警）；
tag v1.2.3 → Release assets=5（acaja.exe 3.05MB / acaja-ui.exe 15.83MB / zip 9.56MB / README×2）。

---


## [2026-09-11] v1.2.2：真·毛玻璃 + 动画流畅度 —— 两个根因都定位到代码/API 层

总目标：用户反馈 ① **完全没有毛玻璃透感**，界面只是个平面 ② **动画生硬**；并询问能否改用网页界面（本轮只做可行性评估）。
用户同时明确：**取消原定功能迭代 2/3/4**，现阶段专注 UI 重度开发。

状态：✅ 完成（CI success：**57 单测**；Release **v1.2.2** 已发，5 资产）

干到哪了：

**A. 根因 1「没有毛玻璃」= 两点，都不是 Rust 的限制**
1. **我们自己的底是 96% 不透明**（`bg_base = rgba(8,8,10,244)`）→ 桌面根本透不进来，玻璃只是「在自己渐变上做半透明」。
   责任在主控上一轮的设计要求（"系统透明不可用时界面也要完整"），subagent 照做。
   修法：**双模式底** —— 系统模糊生效时 `bg_base` alpha 244 → **88（≈35%）**，合成后整窗 alpha≈0.48，桌面真能透进来；
   不生效时保持原不透明底（界面依旧完整）。
2. **系统模糊调用从未生效**：① 旧代码按窗口标题 `FindWindowW` 找句柄，找不到就静默重试、**零诊断**；
   ② 无边框窗口（`decorations(false)`）**没有边框区**，Win11 的 `DWMWA_SYSTEMBACKDROP_TYPE` 必须配合
   `DwmExtendFrameIntoClientArea(-1)` 把玻璃延伸到客户区才可见。
   修法：`windowfx.rs` 重写 —— 句柄改为**枚举本进程顶层窗口**；模糊按「亚克力 `SetWindowCompositionAttribute`
   (Win10 1803+/Win11) → Win11 系统背板(+延伸客户区) → Aero 模糊」**三级回退**并记录结果；新增 `blur_active()`
   供界面决策、`status_line()` 供诊断；修掉窗口圆角漏色；设置界面「系统」分区显示毛玻璃状态；
   主程序 `--diag` 现在透传给设置进程（一条命令同时拿两侧日志）。

**B. 根因 2「动画生硬」= egui 的默认缓动是线性**
`egui 0.30 context.rs:2869`：`animate_bool_with_time` 内部委托 `animate_bool_with_time_and_easing(..., easing::linear)`。
→ 全部改用 `*_and_easing`（cubic_out / expo_out / quadratic_out），数值动画用其返回值 lerp；
按场景拆分时长（悬停 0.14s / 按下 0.09s / 选中 0.26s / 分区 0.32s / 卡片入场 0.42s 错峰 45ms）；
**消除瞬时跳变**（导航前景色/图标/序号、预设行激活态全部改缓动）；开启 `style.scroll_animation` 平滑滚动；
修掉 flash 淡出依赖鼠标事件的 bug；动画结束即停重绘（静止零重绘）。

**C. 性能（流畅度的另一半）**
背板「基底+渐变+光斑+网格+噪点」由每帧约 230 图元改为**一次性 CPU 栅格化成贴图**（键 = 像素尺寸+主题+透底模式，
尺寸变化去抖 140ms；DPI 折进像素尺寸），每帧只画 1 张贴图 + 少量光斑网格；面板阴影 12 → 6 层。

**D. 玻璃厚度**：跟随鼠标的镜面高光（窗口层 + 每张面板，圆角矩形周长顶点扇形，光被形状天然裁住）、
光斑 ±9px / 面板 ±2.6px **反向视差**（只位移形状，不动内容与命中区，布局零抖动）、四边随光向的亮/暗折射带。

踩坑（本轮新增，两条都值得记住）：
- **本地 `cargo check` 查不出链接错误**：`SetWindowCompositionAttribute` 是 user32.dll 的**未公开导出**，
  SDK 的 `user32.lib` 里没有该符号 —— 静态 `#[link(name="user32")] extern "system" { ... }` 声明能过类型检查，
  但 CI 链接报 `LNK2019: unresolved external symbol __imp_SetWindowCompositionAttribute`。
  改为运行时 `GetProcAddress("user32.dll", ...)`（业界通行做法）。再次印证：**编译能过 ≠ 链接能过**，链接与行为只能靠 CI。
- 无边框窗口的系统磨砂必须「设置背板属性 **+** 把 DWM 玻璃延伸到客户区」同时做，否则完全看不到效果（已写进代码注释）。

验证证据：commit f8ec302 → CI success（`test result: ok. 57 passed; 0 failed`，源码零告警）；
tag v1.2.2 → Release assets=5（acaja.exe **3.04MB** / acaja-ui.exe 15.82MB / zip 9.55MB / README×2）。

下一步：见 README「下一步 UI 优化点」。**网页界面（WebView2）可行性评估**结论：
CSS `backdrop-filter` 无法采样桌面（Chromium 视透明窗口背后为空像素），桌面磨砂**只能由系统合成器提供**
——即换成网页也需要我们刚做的这套 OS blur；网页的真实收益是表现力与迭代速度，代价是 WebView2 运行时依赖 + 界面重写
+ 无边框拖拽/DPI 重新处理。建议先做 PoC（`acaja-ui.exe --web`，保留原生界面回退），实测对比后再决定是否迁移。待用户拍板。

---


## [2026-09-11] v1.2.1：视觉精修（迭代 1.5）—— 深黑玻璃拟态 + 衬线×非衬线双字体 + 丝滑动效

总目标：用户对 v1.2.0 的界面给了**新的视觉方向**——① 深色模式、深邃黑灰为主 + 低饱和霓虹紫或克莱因蓝
② 大留白 + 不对称网格 + 玻璃拟态 + 更多毛玻璃透感 ③ hover/enter 动效极其丝滑、不得有生硬闪烁
④ 衬线/非衬线现代搭配、粗细对比明显。功能不动，只重做视觉。

状态：✅ 完成（CI success：**57 单测全过**；Release **v1.2.1** 已发，5 资产）

干到哪了：

**A. 视觉（subagent 实现，主控定设计令牌）**
- 配色：`#08080A`/`#101014`/`#060608` 深黑灰底 + 霓虹紫 `#6C5CE7` + 克莱因蓝 `#002FA7`；
  霓虹紫→克莱因蓝的**色相流动带**铺在玻璃面板里，背景光斑色相能透过每张卡片；文字 `#F4F5F8`/`#A7ABB8`/`#71747F`；状态色压一档饱和。
- 布局：窗口留白 28 / 卡内 20~24 / 分组间 32；**不对称两列网格**（hero 跨列、列宽比 0.58、行高取两列最大值并推进光标）；
  侧栏玻璃轨 200px。
- 毛玻璃（重点）：面板 30 余层叠加 —— 12 层柔化外阴影（alpha 按 (1-t)² 递减）+ 15% 玻璃底 + 色相流动带 +
  56px 顶部光泽 + 1px 镜面边（顶亮底暗）+ hover 紫色柔光/阴影加深；凹陷控件（输入/滑杆槽/色井）单独做内阴影 + 底部厚度高光。
- 动效：hover 的底色/描边/抬升三属性同步过渡（无跳变）、卡片入场 260ms 淡入上移（错峰、只播一次）、分区切换淡入；
  仅在动画收敛前 request_repaint，静止帧不重绘。

**B. 字体（主控，关键使能项）**
- 三套字体从**同一字符集**（GB2312 全汉字 + ASCII + 界面符号，共 7546 码位 → 8206 字形）与同一工具链生成：
  `ACAJASans-Regular.ttf` / `ACAJASans-Bold.ttf`（Noto Sans SC 实例化 400/700）/ `ACAJASerif-SemiBold.ttf`（Noto Serif SC 600），
  三族 `unitsPerEm` 均为 1000 → 行内中英混排基线一致。
- 替换旧的单套 `ACAJACJK-Regular.otf`（1.57MB，覆盖未知）→ 三套共 8.1MB（UI 进程体积用户明确不在乎）。
- `fonts.rs` 改为 **一次 `install(ctx)`** 装好三族；命名族 `serif` / `bold` **恒定存在**（缺字体自动回退无衬线），
  UI 侧不需要任何字体可用性分支（契约先行，避免 subagent 写出条件分支）。
- 单测 +3：三套字体均可被 ab_glyph 解析、字形覆盖（含 ● ✓ → ° · … 等界面符号）、三族度量一致。

**C. 主程序轻量复核（铁律）**
新增 9.5MB 字体资产**只进设置进程**：`acaja.exe` **3.04MB**（v1.2.0 时 3.03MB，无泄漏 —— 后端从不引用字体代码，链接期裁掉）；
`acaja-ui.exe` 9.39 → 15.79MB（可接受，游戏时该进程是关掉的）。

**D. 协作（用户要求用 subagent 设计前端）**
- 主控先定死设计令牌（色值/间距/字号/动效时长/API 契约）再派单；subagent 独占 `src/ui/{mod,preview,strings}.rs`，
  主控独占 `fonts.rs` + 字体资产 + 文档。
- subagent 自建**一次性无头 egui 布局验证器**（macOS 上跑不了 Windows GUI）核对不对称网格与入场动画：
  16/16 不变量通过（零高子 Ui 分栏、两列不等宽不等高、行高推进正确、抬升不引起布局抖动、ScrollArea 内缩与裁剪正确）；
  并顺手修掉一处 `usable<480` 时 `min>max` 会 panic 的 clamp 写法。
- subagent 反向报回主控代码的编译错误（`ab_glyph::Font::units_per_em()` 返回 `Option<f32>`，我在测试里直接 collect 成 `Vec<f32>`）→ 立即修复。

验证证据：commit 263594d → CI success（`test result: ok. 57 passed; 0 failed`，源码零告警）；
tag v1.2.1 → Release assets=5（acaja.exe 3.04MB / acaja-ui.exe 15.79MB / zip 9.54MB / README×2）。

下一步（迭代 2 · v1.2.2，见 README）：每发后坐力曲线（连发累积 / 单发增量 / 上限 / 恢复速率）、
开火来源独立开关（左键 / 手柄 RT）、命中与开火闪烁指示、扩散恢复指示条样式；不做键盘钩子式「按住热键」。

---


## [2026-09-11] v1.2.0：迭代 1 —— 液态玻璃界面 + 档案/编辑效率（功能优先）

总目标（用户三方要求）：① UI 要「现代 + 高级」（Apple 液态玻璃风格，可激进）② 以 **Crosshair X** 为对照做**四轮功能迭代**，
每轮在 README 留下进度与下轮优化点 ③ 铁律：**主程序绝对干净轻量**（不加线程/轮询/系统钩子/依赖，重量级能力只进设置进程），
不做游戏内快速面板。

状态：✅ 完成（CI success：**55 单测全过**；Release **v1.2.0** 已发，5 资产）

干到哪了：

**A. 界面（全部在设置进程 `acaja-ui.exe`，主程序零成本）**
- 无边框 + 透明窗口 + 自绘标题栏（拖动 / 双击最大化 / 自绘 8 向缩放边）；Win11 上叠加 DWM 圆角与亚克力背板
  （`system/windowfx.rs`，失败静默降级——界面自带不透明基底，系统透明不可用时依然完整）。
- 深空背板（圆角渐变 + 三色柔光斑 + 细网格 + 确定性噪点）、玻璃面板（多层偏移阴影 + 镜面高光边 + 同心圆角）、
  统一控件皮肤（按钮/滑杆/勾选/下拉/输入/色块）、几何图标导航（选中光条动画）、弹簧过渡动效；深浅两套高级调色；无 emoji。

**B. 功能（对齐 Crosshair X 的档案与编辑体验）**
- **预设列表化管理**：激活 / 复制 / 重命名 / 删除（内联二次确认）/ 导出 / 导入；重命名自动迁移游戏绑定，导入重名自动序号。
- **预设名安全校验**：名字会落成文件名 → 挡住 `../`、`\`、`:`、保留名（CON/NUL…）、超长、控制字符、结尾空格/点
  （修掉此前可越界写文件的隐患）。
- **热键录制**：点一下直接按组合键即录（含 OEM 符号键），Esc 取消、一键清空 —— 取代手打字符串。
- **显示器选择**：真实分辨率 + 主屏标记 + 一键把准星移到该屏中心；**自定义图片**：原生文件对话框 + 一键清除；
  **模板画廊**：8 套模板缩略图网格，点图即用。
- **实时同步（关键手感升级）**：参数一变即推主程序（节流 140ms + `SendMessageTimeout` 120ms 上限）→ **屏幕上的真实准星即时跟随**
  （Crosshair X 同款手感）。主程序忙（如托盘菜单模态循环）时放弃本次预览而不冻结界面；写盘仍由「应用」/关窗自动保存负责。

**C. 主程序轻量核算（铁律验证）**
本轮对主程序新增 **0 线程 / 0 轮询 / 0 系统钩子 / 0 依赖**；实时同步只是在原有 WM_COPYDATA 路径上提高调用频率，
空闲时主循环仍完全睡眠。产物证据：`acaja.exe` **3.03 MB**（上一版 3.19 MB，反而更小 —— rfd/DWM 只被设置进程用到，链接期被裁掉）。

**D. 协作方式（用户要求「开 subagent 用当前模型设计前端」）**
- subagent（`task` 类型 = 当前会话模型）独占 `src/ui/**`，主控独占 `config.rs` / `ipc.rs` / `system/*` / 文档；
  **先把接口定死再并行**（filedialog / windowfx / config 档案 API），完成后主控集成并补实时同步。
- subagent 反向报回一个真 bug（`vk_to_key_token` 缺 OEM 符号映射 → 标点热键存盘后无法解析），主控修复并加断言。

踩坑 / 修复（**CI 的 `cargo test` 抓到的三个真 bug，本机类型检查抓不到**）：
1. `key_token_to_vk` 只判 `len == 2` → **F10~F24 无法解析回来**（预设 JSON 反序列化整条失败）；改为接受 2~3 字符。
2. `validate_preset_name` 先 trim 再查结尾空格 → `"x "` 这类非法名漏网；改为对原始输入检查。
3. `rename_preset` 在 `delete_preset` 之后判断激活项，而 delete 会把激活名改回 default → **重命名当前预设后激活名丢失**；
   改为删除前记录 `was_active`。
（热键往返用例扩展到 35 个键码，把 F10/F11/F12/F24 这组盲区钉死。）

验证证据：commit 8f80923 → CI success（`test result: ok. 55 passed; 0 failed`）；tag v1.2.0 → Release assets=5
（acaja.exe 3.03MB / acaja-ui.exe 9.39MB / zip 5.95MB / README×2）。

下一步（迭代 2 · v1.2.1，见 README 迭代路线）：每发后坐力曲线（连发累积 / 单发增量 / 上限 / 恢复速率）、
开火来源独立开关（左键 / 手柄 RT）、命中与开火闪烁指示、扩散恢复指示条样式；
明确不做键盘钩子式「按住热键」（违反轻量铁律，鼠标右键/手柄按住模式已覆盖）。

---

## [2026-09-11] v1.1.7：导航空白项根治（i18n 单一表格）+「偶尔报错后直接卡掉」硬化 + 遗留问题清零

总目标：用户反馈两件事——① 准星主程序偶尔报错并直接卡掉；② 设置界面左侧导航有项目显示为「无命名空白」。
另要求：遍历全项目找可优化点、解决遗留问题、补齐 agent 指导文件。

状态：✅ 完成（CI success：48 单测全过，Windows release 构建通过；commit 6b1e209 / d5c4995 / d334f8f / 5116d31 / b985194）

干到哪了：

**A. 导航空白项（根因已定位到代码行）**
- 根因：`nav_ui` 取文案用 `t(lang, "nav_gamepad")` / `t(lang, "nav_image")`，但 `ui/strings.rs` 的 `zh()` / `en()`
  两个**互相独立**的 match 表里都没有这两个 key，兜底 `_ => ""` 直接渲染空串 → 7 项导航里「手柄」「自定义图片」
  两项显示为无命名空白（用户所见即此）。
- 结构性修复：`strings.rs` 由「zh/en 两份独立表」改为**单一表格**（一行 = 一个 key，中文/英文同排，
  按 key 升序 binary_search）→ 漏翻译在结构上不可能发生；key 只出现一次，也不会两边写岔。
- 导航标签与卡片标题**解耦**：卡片标题很长（「手柄（Apex 瞄准吸附）」），148px 导航列放不下，
  新增 `nav_*` 短标签，key 清单统一在 `src/ui/mod.rs::NAV_ITEMS`（8 项）。
- 新增 4 个回归单测：导航 key 中英双语非空、形状/ADS 模式名全覆盖、表有序唯一、每行双语非空；
  删除 9 个无调用点的死 key（subtitle/close_quits/backend_disconnected/language/theme/show/hide/quit/hex_hint）。
- 顺带：托盘菜单随界面语言本地化（此前写死中文），预览区「自定义图片」提示不再写死 `Lang::Zh`。

**B.「偶尔报错后直接卡掉」（发行版是 windows_subsystem="windows" 且默认不写日志 = 静默死亡，此前无从诊断）**
静态排查出 5 个真实致死/致残点，全部修掉：
1. **崩溃不可诊断** → panic 钩子（主程序 + 设置进程）恒写 `%APPDATA%/ACAJACrosshair/acaja-crash.log`，不受 `--diag` 限制。
2. **一次 panic = 进程消失** → 主消息循环每次迭代 `catch_unwind` 隔离，捕获后继续运行（准星不掉线），首次弹一次非阻塞提示。
3. **锁中毒放大** → `.lock().unwrap()` 在持锁 panic 后永久 Err，把「一次小 panic」放大成「之后每次都 panic」的砖头；
   现改用 `parking_lot`（lock()/read()/write() 直接返回 guard，无 PoisonError 路径）。
4. **GDI/DIB 泄漏（真·内存增长）** → `DeleteObject` 对「已选入 DC 的位图」会失败，而 `ensure_canvas_size`
   每次画布尺寸变化都删旧 DIB → 拖动「大小」滑杆每次泄漏一张最大 4MB 的 DIB，长时间使用内存持续上涨；
   现删除前先 `SelectObject` 换出原 stock 位图（`create_dib` 返回并保存该句柄）。
5. **托盘菜单卡住** → `TrackPopupMenu` 前未把宿主窗口设为前台窗口（MSDN 明确要求），菜单可能收不到输入、
   点别处不消失，用户现象即「托盘卡住」；补 `SetForegroundWindow` + 关闭后 `PostMessage(WM_NULL)`；
   且不再在主消息线程弹模态框（会卡住托盘/热键/实时推送，改为独立线程弹）。
- 其他：overlay/gamepad 线程创建失败改为降级 + 告警（不再 `expect` 崩进程）；空多边形不再让渲染线程越界 panic；
  消息窗口创建失败会明确提示（此前表现为「准星在、托盘热键全没反应」且无声）。

**C. 遗留问题清零（后台已实现、界面碰不到的功能全部接通）**
- 导航新增「系统」分区；新增：**开机自启开关**（注册表 Run 键，路径从主程序窗口反查 → 程序改名/移动也写对目标；
  此前 `set_autostart` 取 `current_exe()`，从设置进程调用会写成 acaja-ui.exe，等于开机只弹设置窗）、
  **窗口吸附开关**（`snap_to_window`，后台早已响应前台窗口移动）、**「切换下一预设」热键输入框**（后台早已注册
  `hotkey_next_profile`）、**游戏绑定编辑器**（前台进程 → 自动切预设，加/删/去重）。
- 删除死代码：`i18n::Strings` 双份文案表（v1.1.0 双进程重构后已无调用点，托盘文案也因此停留在旧版）、
  从未被读取的配置字段 `auto_topmost` / `minimize_to_tray`、未使用的常量/导入/字段/函数；
  构建告警 **40+ → 0**。

**D. agent 指导文件**
- 新增 `AGENTS.md`：架构地图、验证方式（CI + 本地交叉 `cargo check` 的确切命令）、代码约定、已验证的坑清单、当前边界。
- 本 WORKLOG 继续作为开发日志；README/README_CN 同步到 v1.1.7 现状（含「两个 exe 必须同目录」等 FAQ）。

踩坑（本轮新增）：
- **「未使用变量」≠ 可删**：编译器提示 `store` 未使用，删掉后 CI `cargo test` 报
  `corrupt_file_falls_back_to_defaults` panic——那次 `PresetStore::open` 的**副作用**是创建 `presets/` 子目录。
  本地 `cargo check` 只做类型检查，抓不到这类行为回归 → **行为门禁只能是 CI 的 cargo test**。
- 交叉类型检查需要目标 rust-std（装了 `x86_64-pc-windows-msvc` 的 rust-std 到现有 toolchain），
  且 build.rs 在没有 rc.exe 的主机上必须能跳过 winres → 新增 `ACAJA_SKIP_WINRES=1` 开关。

验证证据：commit b985194（UI/i18n）→ 5116d31（后台硬化）→ d334f8f（单测修复）→ d5c4995（parking_lot）→ 6b1e209（文档）；
GitHub Actions **success**，`test result: ok. 48 passed; 0 failed`，Windows release 构建通过。
本地 `cargo check --all-targets --target x86_64-pc-windows-msvc` 通过（并已用「故意注入类型错误」验证该通道真能报错）；
注：本机 toolchain 较旧，**未覆盖新版 lint**，告警情况以下方第二轮补充的 CI 结论为准。

补充（同日 · 第二轮）：
- **f32 回退 lint 修复**：CI 报 14 处 `egui::Stroke::new(1.0, …)` —— `impl Into<f32>` 参数收到未标注浮点字面量，
  新版 rustc 报 `falling back to f32 as the trait bound f32: From<f64> is not satisfied`（**将来会变成硬错误**），
  全部改为 `1.0_f32`。
- **排查失误记录（值得记住）**：一度误判「零告警」，两个原因 —— ① 筛告警用 `^warning`（行首），
  而 `--message-format short` 的格式是 `文件:行:列: warning: …`，全部漏掉；② `gh run view --log` 会把转义符写成
  **字面量 `^[`**（不是 ESC 字节），不先替换就匹配不到 `-->` 位置行。
  另：本机 rustc（1.92.0，2025-12）比 CI 的 stable 旧 → **新版 lint 只在 CI 出现**，「本机零告警」不等于干净。
- **发版规则落地**：`AGENTS.md` 新增 §2.1（每次更新完成、CI 变绿后立即按序打 tag 发 Release）；本轮按该规则发 v1.1.7。
- **旧版 Python 产物下线**：取消跟踪 `小林の准星.exe`（48MB）与 7 个 PySide6 源码 + `requirements.txt`
  （用户确认无保留价值）；跟踪文件 49 → 40。`.git` 仍 111MB（旧 blob 在历史与 v1.0.x/v1.1.x tag 中，
  需重写历史才能释放，另行决策）。

边界与待用户实测：
- 用户那台机器上「偶尔卡掉」的**具体触发路径无法在 macOS 复现**（无 Windows 运行环境）：本轮做的是
  ① 消除所有静态可证的致死路径；② 让之后任何内部错误都留痕（acaja-crash.log）。
  若再出现，请把 `%APPDATA%/ACAJACrosshair/acaja-crash.log` 发回，即可定位到具体行。
- 未做：自定义图片文件选择器（仍手填路径）；独占全屏覆盖（系统限制，任何 overlay 都不行）。

---

## [2026-08-28] v1.1.6：现代化 UI 重设计（导航化布局）+ 外部 UI 模型通道评估

总目标：用户要求「重新设计 UI 更现代化」。尝试 glm-5.3（ui-designer agent）两次：Stream ended / Connection error——pi 的 subagent 模型流在该环境不可靠（模型调用不走用户 shell 代理，代理方案对模型链路无效；用户 shell 代理 env 已确认 127.0.0.1:7890）。判定：外部模型通道不可用，由主控自行完成设计。

状态：✅ 完成（v1.1.6 CI success）

干到哪了：
- **布局重构**：品牌顶栏（几何 A 方块 + 标题版本 + 语言/主题）→ 左侧 7 项导航（几何状态点/强调条，无 emoji）→ 右侧卡片区（样式/动态/位置/手柄/快捷键/图片/预设 分区显示，Card = 圆角 10 + 分层底色）→ 底部固定操作条（连接状态 +「应用设置到主程序」主按钮 +「退出主程序」）。
- **深色分层主题**：自定义 Visuals（BG #14161c / CARD #1f232d / 边框 #2a2f3a / accent #0a84ff soft #102e52）；文字色 #e5e7ed；次要文本 #8a90a0。
- **保留全部逻辑**：预设 CRUD/模板/热键/手柄（ADS 四模式+LB/RB）/右键三模式/推送按钮/退出命令/dirty 标记/退出自动保存。
- 踩坑：egui 0.30 `hline(x: Rangef, y, stroke)` 3 参（写错 4 参）；`painter.rect_filled` 等 OK；Nav 中 `preset.dirty` 笔误。
- 验证证据：commit 5b305fd → CI success（45+ 单测）；v1.1.6 release assets=5（acaja.exe/acaja-ui.exe/zip/README×2）。

下一步：实测新 UI；Cargo version 同步 1.1.6 重打 tag（zip 名修正）。

---
## [2026-08-28] v1.1.1：双二进制拆分 + MsgWait 事件驱动（常驻极致轻量化）

总目标：用户明确「接受多线程/多进程，只要轻量化、低内存、低 CPU、发挥 Rust 优势」→ 常驻进一步压缩。

状态：✅ 完成（v1.1.1 CI success；release 4 资产）

干到哪了：
- **双 bin 拆分**（Cargo.toml `[[bin]]`）：`acaja.exe` = 后台壳（链接裁剪 egui/eframe/glow → 2.8MB，v1.1.0 单 exe 双入口时后端仍带 egui 代码）；`acaja-ui.exe` = 设置进程（8.4MB）。`spawn_ui_process` 优先同目录 acaja-ui.exe。后端进程映射/驻留页大幅下降。
- **MsgWaitForMultipleObjectsEx 事件驱动**：消息循环从「50ms 定时轮询」改「消息就绪即唤醒 + 50ms 兜底」；手柄线程事件到达时 PostMessage(WM_APP+66) 即时唤醒（HWND 跨线程传 isize 规避 Send 限制）；WM_COPYDATA（UI 实时改参）SendMessage 同步进队即刻唤醒 → UI 拖动滑块仍流畅；空闲时线程完全睡眠（CPU ≈ 0）。
- **workflow 打包两个 exe**（ACAJA-vX + ACAJA-UI-vX）。
- 验证证据：commit a20a753 → CI success；v1.1.1 release 4 资产（壳 2.8MB / UI 8.4MB）。

预期成效：后台壳常驻内存 ~10-18MB（比 v1.1.0 再降），空闲 CPU ≈ 0%（手柄轮询除外 ~0.5%）；设置进程用完即走，内存峰值仅在打开时。

下一步：用户实测数值。README 打包/架构说明已含双进程（补双 exe 说明可后续）。

---
## [2026-08-28] v1.1.0：设置界面拆分为独立进程（双进程架构，内存对齐 RTSS 级别）

总目标：用户要求「独立 UI 进程」成品——后台壳 ~15MB 常驻，设置界面按需拉起、关闭即彻底释放（~65MB）。

状态：✅ 完成（v1.1.0 CI success）

干到哪了：
- **架构**：`acaja.exe` 默认 = 后台壳（D2D 覆盖层 + 输入 + 托盘 + 热键 + 前台检测；不再链接 eframe）；`acaja.exe --ui` = 设置进程（egui，独立互斥体 "ACAJACrosshairSettings" 单例）。
- **IPC**：`src/ipc.rs` — WM_COPYDATA 同步通道（windows-rs 0.58 未绑定 COPYDATASTRUCT，手工 repr-C 定义）；负载 = `{"v":visible,"p":<preset json>}`；UI 帧末 33ms 节流推送；主进程 WM_COPYDATA 分支 → 反序列化 → shared/hotkey/gamepad_cfg/overlay 全链路同步。
- **进程生命周期**：主进程托盘「打开设置」/启动即 spawn `--ui`；UI 点 X = 保存 + 本进程退出（主进程不动）；UI 每 2s FindWindow("ACAJABackend") 自检，主进程退出/崩溃 → UI 自动退出。主进程消息窗口标题定名 "ACAJABackend" 作为 FindWindow 依据（UI 窗口标题仍 "ACAJA"）。
- **清理**：ui/mod.rs 删除 overlay/shared 耦合与全部静态（UI_CTX/UI_OPEN/UI_HIDDEN/QUIT_REQUESTED/show_settings_window/后台按钮），push_to_overlay → send_to_backend；main.rs 全重写（backend_process_main + ui_process_main 分流），事件驱动 select 循环保留。
- 验证证据：commit 8b73f2d → CI success（45+ 单测）。

下一步：用户实测 v1.1.0——后台壳内存（预期 ~15MB）与设置窗关闭彻底释放；托盘开设置实时同步。README 架构节待同步双进程。

---
## [2026-08-28] v1.0.11：后台=真隐藏（任务栏干净）+ 托盘右键修复 + 右键切换/开火真正接线

总目标：① 后台运行不再留任务栏标签（最小化被否）② 托盘右键菜单无反应 ③ 鼠标右键点击/长按隐藏准星功能“缺失”（实际是功能从未接线）④ 内存进一步优化给出路径。

状态：✅ 完成（v1.0.11 CI success，release 已发）

干到哪了：
- **根因复盘**：v1.0.10 最小化版被用户否掉（任务栏有标签）；恢复「真隐藏」（Visible(false)）+ **UI_HIDDEN 自有标志压制渲染 ≤1fps**（隐藏期每帧 request_repaint_after(1s) → 渲染频率 ≤1fps，从根源杜绝 v1.0.8 的 6% 空转；egui 0.30 的 ViewportInfo.visible 通过 ctx.input 不可得，改用自有 AtomicBool 更稳）。
- **托盘右键菜单修复**：Shell_NotifyIcon 日常回调是 WM_RBUTTONUP(0x205) 而非 WM_CONTEXTMENU → 补上（两者都处理）。这是“点右键没反应”的根因。
- **右键切换/左键开火功能接线**：pump_message_batch 消息泵长期遗漏 WM_INPUT 分支——Raw Input 只在启动时注册，事件从未消费。补上：RightDown/Up → on_right_button（点击切换/按住显示/按住隐藏三模式即刻可用）；LeftDown → fire_started（开火扩散）。
- 踩坑：ViewportInfo.visible 字段在 eframe/egui 0.30 间接访问失败（input().viewport() 不可见字段），改自有标志；main.rs import 两次部分替换遗漏。
- 验证证据：commit 104bd34 → CI success；v1.0.11 release assets=3。

下一步：用户实测（后台=任务栏无标签+CPU 低；托盘右键弹菜单；右键点击/长按隐藏）。内存：egui 框架常驻 ~65MB 是结构性成本（对比 RTSS 属原生 C++ 无 UI 框架）；v1.1 若做「UI 独立进程」（后台彻底卸载 UI，内存落 ~15MB）可对齐 RTSS，待用户确认是否值得投入。

---
## [2026-08-28] v1.0.10：后台运行改为最小化（v1.0.9 关窗卡死、v1.0.8 隐藏 6% 的终结方案）

总目标：找一个「后台模式」的稳定低占实现。实证：隐藏窗口（Visible(false)）→ 失去 vsync 节流 → 渲染空转 6%（v1.0.8）；真正 Close → eframe 清理流程在某环境卡死（v1.0.9 用户实测强制退出）。

状态：✅ 完成（v1.0.10 CI success，release 已发）

干到哪了：
- **结论**：窗口「最小化」（ViewportCommand::Minimized）保持窗口可见状态 → vsync/事件驱动行为与「设置窗开着」完全一致（开着时用户实测 CPU 很低）→ 无渲染空转、无关闭清理路径。
- **改动**：「后台运行」按钮 = 保存设置 + Minimized(true)；托盘「打开设置」= Minimized(false) + Focus；main.rs 移除 BACKGROUND_REQUESTED/reopen 全链路（恢复 X=退出直达路径）；on_exit 自动保存保留。
- 验证证据：commit a3cc331 → CI success；v1.0.10 release assets=3。

下一步：用户实测后台最小化后 CPU（预期≈设置窗开着时）与不卡死。

---
## [2026-08-28] v1.0.9：后台运行=真关窗（解决隐藏后 CPU 6%）+ 发行版关闭日志

总目标：① 修「点后台运行后 CPU 回到 6%」（隐藏窗口的渲染循环仍空转）② 发行版不生成日志文件。

状态：✅ 完成（v1.0.9 CI success，release assets=3）

干到哪了：
- **根因**：ViewportCommand::Visible(false) 隐藏窗口后 eframe 渲染循环仍以 60fps 后台空转（隐藏窗口未停渲染）。用户在设置窗开着时测不到（事件驱动低占用），点后台运行反而暴露。
- **修复**：「后台运行」= 置 `BACKGROUND_REQUESTED` + 真正 Close（释放全部 egui/GL）；main.rs 恢复 UI 生命周期循环：X=退出 / 后台运行=关窗后等待 recover（托盘「打开设置」→ reopen → 重建窗口）或 quit（托盘退出）。msg_thread_main 与 pump_message_batch 增加 reopen 参数（修复脚本替换伤到 pump_message_batch 的编译错误）。
- **日志**：`init_logging` 在 `cfg!(debug_assertions)` 条件下才建文件日志 → release 版不产生 acaja.log；debug 构建保留。
- 验证证据：commit a997c78 → CI success；v1.0.9 release 3 资产。

下一步：用户实测后台运行 CPU ~0%；readme 待同步「后台运行=关窗」语义。

---
## [2026-08-28] v1.0.8：LB/RB 触发动态生效 + CPU 事件驱动化（6% → <1%）

总目标：① 修"设了 LB 仍扳机触发"（触发源是启动参数，UI 改动不生效）② CPU 6% 降到最小。

状态：✅ 完成（v1.0.8 CI success，Release assets=3）

干到哪了：
- **手柄配置动态化**：`input/gamepad.rs` 新增 `RuntimeGamepadCfg{threshold, ads_source}`（Copy + RwLock 共享）；`start_gamepad(cfg)` 轮询线程每 8ms 读取最新配置 → UI 里改 LB/RB/阈值**即时生效**（此前启动即固定，v1.0.6 修了位常量但同步链路漏了）；msg 线程 `sync_from_ui` 在 shared preset 版本变化时同步 `gamepad_cfg`。
- **CPU 优化（事件驱动）**：消息线程从"try_recv 轮询 + sleep"（每秒 20-100 次空转）改为 `crossbeam select!`：gamepad/fg 通道**阻塞式实时接收**（事件零延迟），消息泵（托盘/热键）只在低频 tick（UI 开 10ms / 关 50ms）执行；XInput 轮询 4ms→8ms（125Hz，ADS 延迟 <8ms）。预估常驻 CPU <1% 单核。
- 踩坑：大段主循环替换脚本分 A-G 步骤，其中 A/B 段首轮失败未落盘导致编译错（start_gamepad 参数/签名不一致），拾遗修复；`RwLock` import 补漏；tag 触发的 release 异步生成延迟（gh 查询过早显示 404）。
- 验证证据：commit d9968af → CI success；v1.0.8 release assets=3。

下一步：用户实测（LB 触发 + CPU 数值）。backlog：v1.1 = 托盘重开设置（UI 已退出场景）、Raw Input HID 手柄（零轮询）可选深度优化。

---
## [2026-08-28] v1.0.6 + v1.0.7：LB 肩键位修正 / 中心语义统一 / 关窗=保存并退出 / 自动恢复设置

总目标：修复肩键触发、松手位置偏移；按用户操作直觉重构退出路径（点 X = 全退）+ 设置自动保存。

状态：✅ 完成（v1.0.7 CI success，Release 已发 assets=3）

干到哪了：
- **v1.0.6**：① 肩键位常量修正 `0x0100/0x0200`（此前误写 0x0004/0x0008=十字键 LEFT/RIGHT——手写常量失误，CI 检测不到）；② 位置语义统一：`state::resolve_position(&Preset)` = 显示器「几何中心」（非工作区中心）——修复「按住隐藏/松手后准星向上偏移」（UI 用几何中心而消息线程用工作区中心，语义不一致导致事件回跳）。main.rs position_for_preset 与 ui push_to_overlay 全部改用统一函数。
- **v1.0.7（用户实测反馈）**：点 X 之前是"窗口关但程序常驻"（用户不满意，只能任务管理器）。重构：**点 X = 自动保存设置 + 彻底退出**；新增「后台运行」按钮 = 隐藏窗口常驻（托盘可重开设置/退出）；**on_exit 自动保存 working copy + app.json** → 下次打开软件沿用上次设置（原先必须手动点保存）。show_settings_window 隐藏状态恢复 Visible+Focus。
- 踩坑：sed 不走 macOS（unterminated substitute），继续用 python 替换；ui/mod.rs 缺 `use log::warn` import。
- 验证证据：v1.0.6/v1.0.7 各自 CI success；v1.0.7 release 3 资产。注：v1.0.4/v1.0.6 未单独打 tag，其内容已并入 v1.0.7 发布。

下一步：用户实测 v1.0.7（X=退出+保存恢复；LB 触发；松手位置不再偏移）。待办 backlog：托盘「打开设置」在 UI 已退出场景无窗口可开（v1.1 可做重启进设置模式）；readme 状态同步。

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