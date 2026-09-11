# ACAJA — Crosshair Overlay for Windows

> **Current version v1.2.2**: full feature set (settings UI / tray / hotkey / gamepad ADS / per-game auto profiles).
> Maintainers: see [AGENTS.md](AGENTS.md) (architecture map, how to verify, known pitfalls) and [WORKLOG.md](WORKLOG.md) (change log).

ACAJA is a Windows desktop crosshair overlay written in **Rust** (`windows-rs` + Direct2D + egui) — a complete rewrite of [CrossHairLIN](https://github.com/liuroland55/CrossHairLIN) (Python/PySide6). Click-through, zero runtime dependencies.

**Key improvements over the Python version**: ~8 MB exe (was 46 MB), event-driven crosshair-sized overlay (≈0% CPU idle), Direct2D GPU rendering, **22 shapes** with quad-color support, modern egui settings window, gamepad ADS support, per-game auto presets.

---

# 📖 User Guide

## 1. Download & Run
1. Grab `ACAJA-v1.2.2-x64.zip` from [Releases](https://github.com/Aacaja/ACAJACrosshair/releases) (or the latest Actions artifact).
2. Unzip and run `acaja.exe`. You get: the **settings window** (dark UI), a **red cross** at screen center (default), and a **tray icon**.
3. Closing the settings window keeps the app running — the crosshair stays, tray controls everything.

> ⚠️ **Both exes must sit in the same folder**: `acaja.exe` (backend) and `acaja-ui.exe` (settings window).
> If only the backend was copied, "Open settings" reports that `acaja-ui.exe` is missing.

## 2. Customizing the crosshair
The settings window has a left-hand section nav, content cards on the right, and a sticky "Apply to main program" bar at the bottom. Everything applies **live**:

| Section | What you can do |
|---|---|
| **Style** | Live preview, 8 style templates (Apex/Valorant/CS2/sniper…), **22 shapes**, size, thickness, opacity, rotation, **multicolor quad mode**, hollow gap, center dot, outline |
| **Dynamic** | Fire spread (px), recovery speed (ms/px), recoil indicator |
| **Position** | Center button, X/Y fine-tune, monitor index, **snap to foreground window** |
| **Gamepad** | ADS mode (Hold-hide / Toggle / Hold-show / Off), trigger (triggers or LB/RB bumpers), threshold 0-255, fire-spread on RT |
| **Hotkeys** | Toggle hotkey, **next-preset hotkey**, right-click modes |
| **Image** | Custom PNG path + scale (select the Custom Image shape first) |
| **Presets** | Save / create (clone) / delete / switch presets, plus the **game-binding editor** (foreground app → preset) |
| **System** | Start with Windows (per-user registry Run key) |

Workflow: tweak → **Apply to main program** (or just close the window — it saves automatically).

## 3. Gamepad (Apex and similar)
1. Connect a gamepad (Xbox-lineage native; PS pads need DS4Windows).
2. Gamepad section → ADS mode: **Hold-hide** (recommended): hold the aim input → crosshair hides; release → shows. **Toggle** per pull; **Hold-show** reversed; **Off** disables.
3. Trigger source: left/right trigger (analog, threshold-based) or LB/RB bumpers (digital).
4. RT fire drives the dynamic spread if enabled.

## 4. Per-game auto profiles
When the foreground window switches to a bound game exe, ACAJA auto-loads that game's preset.

Configure it in the UI (Presets → Game bindings: type the process name, pick a preset, "Add binding"), or edit `%APPDATA%/ACAJACrosshair/app.json` directly:

```json
{ "game_bindings": [ { "exe": "r5apex.exe", "preset": "apex" } ] }
```

(exe name is case-insensitive; preset files live in `%APPDATA%/ACAJACrosshair/presets/`)

## 5. Global hotkeys
Two fields in the Hotkeys section: **toggle crosshair** and **next preset**. Format: `Ctrl+F1`-style (Ctrl/Alt/Shift/Win + F1-F24/letters/digits/Space). Each preset stores its own hotkeys; registration happens as soon as you save (a combination already owned by another app is ignored).

## 6. Tray
- Double-click tray icon: show/hide crosshair.
- Right-click: menu (toggle / open settings / quit) — labels follow the UI language.
- Closing the settings window does NOT quit. Tray **Quit** (or the settings' "Quit main program") exits.

## 7. Legacy migration
First run auto-migrates legacy CrossHairLIN configs from `%APPDATA%/CrosshairApp/` (full field mapping); the old folder is kept.

## 8. FAQ
- **Crosshair missing?** Check the tray icon still exists → double-click to re-show; check the log.
- **"acaja-ui.exe not found" when opening settings?** Keep both exes in the same folder.
- **Tray menu unresponsive / won't dismiss?** Fixed in v1.1.7 (the host window is made foreground before the menu is shown).
- **Backend occasionally errored out and died?** Since v1.1.7 internal errors are caught, the crosshair recovers automatically, and a crash log is always written to `%APPDATA%/ACAJACrosshair/acaja-crash.log` — please report that file if it keeps happening.
- **Covered in the game?** Use **borderless windowed** mode. Exclusive fullscreen (D3D) can never be overlaid by any overlay tool.
- **Gamepad unresponsive?** XInput mode required; PS pads via DS4Windows; lower the threshold.
- **Need verbose logs?** Run `acaja.exe --diag` → `%APPDATA%/ACAJACrosshair/acaja-diag.log`.

---

# Architecture (two processes since v1.1.0)

```
Backend shell (acaja.exe, ~15 MB resident, no UI framework)
├── render thread  : D2D overlay (event-driven, blocks when idle ≈0% CPU)
├── message thread : Win32 pump (tray / hotkeys / RawInput mouse / foreground hook / gamepad events)
└── gamepad thread : XInput polling (125 Hz)

Settings process (acaja-ui.exe, launched on demand, fully released on close)
└── egui window; parameters pushed live to the backend via WM_COPYDATA
```

```
src/
├── bin/backend.rs  # backend entry (overlay/tray/hotkeys/input/main message loop)
├── bin/ui.rs       # settings entry (single instance + backend liveness check)
├── ipc.rs          # cross-process WM_COPYDATA payload
├── config.rs       # preset schema, atomic JSON, legacy migration, hotkey parser
├── overlay/        # D2D renderer (22 shapes, quad colors, outline, rotation, spread, images)
├── input/          # XInput gamepad + RawInput mouse
├── system/         # tray / hotkey / foreground detect / monitors / autostart
├── ui/             # egui settings window (preview, i18n, themes, presets, bindings)
└── state.rs        # ADS state machine, preset cycling, position resolution
```

## Requirements
- Windows 10 / 11 (x64). No runtime dependencies.

## Roadmap (current phase: **heavy UI development**)

> Design law: **the backend `acaja.exe` stays absolutely clean and lightweight** — no UI framework, no extra threads/polling,
> no system-wide hooks, no new dependencies. Heavy lifting lives only in the settings process `acaja-ui.exe` (launched on
> demand, cost doesn't matter). No in-game quick panel.
>
> The user **cancelled** the previously planned feature iterations 2/3/4 (dynamic crosshair, multi-monitor integration,
> usability polish); the settings UI is now the focus.

| Stage | Version | Theme | Status |
|---|---|---|---|
| 1 | v1.2.0 | Profile & editing workflow (presets / hotkey recording / monitors / image picker / template gallery / live sync) | ✅ released |
| 1.5 | v1.2.1 | Visual revision: deep black-grey glassmorphism + serif/sans pairing + silky motion | ✅ released |
| 1.6 | **v1.2.2** | **Real frosted glass**: system acrylic + genuinely translucent base + mouse specular/parallax/edge refraction + eased motion + backdrop pre-bake | ✅ this round |
| ~~2–4~~ | — | ~~dynamic crosshair / multi-monitor / usability polish~~ | ❌ cancelled by the user |

### Stage 1.6 (v1.2.2) — shipped

- **Real frosted glass** (root cause of the previous "flat" look): the window's own base layer went from 96 % opaque to
  **~35 % translucent** (composite alpha ≈ 0.48) and system blur is now actually enabled — acrylic
  (`SetWindowCompositionAttribute`, Win10 1803+/Win11) → Win11 system backdrop (with the DWM glass extended into the client
  area, otherwise an undecorated window shows nothing) → Aero blur, in that order, with an **opaque fallback** when the OS
  offers none (the UI stays complete either way). Also fixed colour leaking through the rounded window corners.
- **Glass thickness**: cursor-following specular highlight (window-wide + per panel, clipped to the rounded shapes),
  reversed parallax (−9 px blobs / −2.6 px panels) and edge refraction bands that follow the light direction.
- **Motion no longer stiff** (root cause: `animate_bool_with_time` uses **linear** easing internally): switched to
  `*_and_easing` (cubic_out / expo_out / quadratic_out) with a per-scene timing table, removed the instant jumps
  (nav foreground, preset activation), enabled `scroll_animation` smooth scrolling, fixed a flash fade-out that waited for
  mouse events, and stopped repainting once animations settle (zero repaints when idle).
- **Performance** (the other half of smoothness): the backdrop is now rasterised once into a texture on the CPU
  (rebuilt on size/theme/translucency change, debounced) instead of ~230 shapes per frame; panel shadows went 12 → 6 layers.
- The System section shows the **glass status** (acrylic / Win11 backdrop / Aero / opaque fallback) so you don't need logs.

### Next UI work (pending your direction)

1. **On-device feedback**: glass intensity (transparency alpha), highlight/parallax strength, animation timings — all one-line tunables.
2. If system blur is unavailable on your machine: alternative "capture the desktop behind the window and blur it ourselves".
3. **Optional WebView2 web-UI PoC** (feasibility notes in WORKLOG): CSS `backdrop-filter`, blend modes and easing control far
   exceed what egui offers; the cost is a WebView2 runtime dependency, a full UI rewrite, and it still needs the OS blur for
   real desktop frost.
4. Continued polish: scroll inertia, section-switch motion, slider drag physics, repaint cost while dragging the window.

### Iteration 1 (v1.2.0) — shipped

**Features**
- **Profile list management**: activate / duplicate / rename / delete (inline confirm) / export JSON / import JSON;
  renaming migrates game bindings that referenced the old name, importing auto-suffixes on name conflicts.
- **Preset-name validation**: names become filenames, so illegal names (`../`, `\`, `:`, reserved `CON`, too long, control
  chars) are rejected with an inline hint — no more writes outside the config folder.
- **Hotkey recording**: click the field and press the combination (Ctrl/Alt/Shift/Win + letters/digits/F1-F24/arrows/symbols);
  Esc cancels, one-click clear. No more typing `Ctrl+F1` by hand.
- **Monitor picker**: real resolution + primary marker per monitor, one click to move the crosshair to that screen's center
  (`-1` = follow the foreground window).
- **Image file picker** for custom crosshairs (native dialog) and a clear button.
- **Template gallery**: the 8 style templates became a thumbnail grid — click to apply.

**UI**
- Apple Liquid Glass: frameless window + custom draggable titlebar, deep-space backdrop with soft light blobs, glass panels
  with specular rim highlights, concentric radii, hover lift, spring transitions, refined dark/light palettes; on Windows 11
  the window additionally gets system rounded corners + acrylic backdrop (graceful fallback — the UI is self-contained).

### Iteration 1.5 (v1.2.1) — visual revision, shipped

- **Palette**: deep black-grey base (`#08080A` / `#101014` / `#060608`) with low-saturation neon violet `#6C5CE7`
  and Klein blue `#002FA7` (a violet→Klein hue flow shows through every card); text `#F4F5F8` / `#A7ABB8` / `#71747F`.
- **Layout**: generous whitespace + an **asymmetric two-column grid** (hero card spans both columns, 0.58 column ratio),
  200 px glass sidebar rail.
- **Glassmorphism**: panels are built from 30+ layers — 12-layer soft outer shadow, 15% glass fill, hue-flow band,
  56 px top gloss, 1 px specular rim (bright top / dark bottom), hover violet bloom; inset controls (inputs, slider
  tracks, colour wells) get their own inner shadow + bottom thickness highlight.
- **Typography**: three same-source fonts (Noto Sans SC regular/bold + Noto Serif SC semibold, 8206 glyphs each,
  identical metrics) — serif headings, bold numerals, sans body; no baseline jumping in mixed CJK/Latin lines.
- **Motion**: hover transitions colour, stroke and lift together (no popping); cards fade+rise 260 ms on entry
  (staggered, played once); section switches cross-fade; idle frames don't repaint.

> Backend check: the ~9.5 MB of new font assets ship only in the settings process — `acaja.exe` stays at **3.04 MB**.

## License
MIT. Original author: 林晓CCC — Bilibili: https://space.bilibili.com/622769073
