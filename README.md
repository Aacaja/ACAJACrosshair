# ACAJA — Crosshair Overlay for Windows

> **Current version v1.0.1**: full feature set (settings UI / tray / hotkey / gamepad ADS / per-game auto profiles).

ACAJA is a Windows desktop crosshair overlay written in **Rust** (`windows-rs` + Direct2D + egui) — a complete rewrite of [CrossHairLIN](https://github.com/liuroland55/CrossHairLIN) (Python/PySide6). Click-through, zero runtime dependencies.

**Key improvements over the Python version**: ~8 MB exe (was 46 MB), event-driven crosshair-sized overlay (≈0% CPU idle), Direct2D GPU rendering, 14 shapes with quad-color support, modern egui settings window, gamepad ADS support, per-game auto presets.

---

# 📖 User Guide

## 1. Download & Run
1. Grab `ACAJA-v1.0.1-x64.exe` from [Releases](https://github.com/Aacaja/ACAJACrosshair/releases) (or latest Actions artifact).
2. Double-click. You get: the **settings window** (dark UI), a **red cross** at screen center (default), and a **tray icon**.
3. Closing the settings window keeps the app running — the crosshair stays, tray controls everything.

> Troubleshoot via log: `%APPDATA%/ACAJACrosshair/acaja.log`

## 2. Customizing the crosshair
Everything is edited in the settings window and applies **live** to the preview and the on-screen crosshair:

- **Shape & Style**: 14 shapes (cross, dot, square, circle, hollow cross/square/+dot, chevron, triangle, V, T, brackets, gap hair, custom image), size, thickness, opacity, rotation, **multicolor quad mode** (independent top/bottom/left/right colors), hollow gap, center dot, outline.
- **Dynamic**: fire spread (px), recovery speed (ms/px), recoil indicator.
- **Position**: center button, X/Y fine-tune, monitor index (multi-monitor).
- **Custom image**: path + scale (select the Custom Image shape first).
- **Hotkey**: toggle hotkey (e.g. `Ctrl+F1`), right-click toggle modes.
- **Gamepad**: ADS mode (Hold-hide / Toggle / Hold-show / Off), trigger (LT/R2), threshold 0-255 (default 30), fire-spread on RT.
- **Presets**: save / create (clone) / delete / switch.

Workflow: tweak → **Save Preset** → done. Unsaved changes live in memory only.

## 3. Gamepad (Apex and similar)
1. Connect a gamepad (Xbox-lineage native; PS pads need DS4Windows).
2. Gamepad section → ADS mode: **Hold-hide** (recommended): hold LT/L2 → crosshair hides; release → shows. **Toggle** per pull; **Hold-show** reversed; **Off** disables.
3. RT fire drives the dynamic spread if enabled.

## 4. Per-game auto profiles
When the foreground window switches to a bound game exe, ACAJA auto-loads that game's preset. Edit `%APPDATA%/ACAJACrosshair/app.json`:

```json
{ "game_bindings": [ { "exe": "r5apex.exe", "preset": "apex" } ] }
```

(exe case-insensitive; preset files live in `%APPDATA%/ACAJACrosshair/presets/`)

## 5. Global hotkeys
Enter `Ctrl+F1`-style strings in the Hotkey section (Ctrl/Alt/Shift/Win + F1-F24/letters/digits/Space). Each preset stores its own hotkeys; toggle & next-preset hotkeys work in-game.

## 6. Tray
- Double-click tray icon: show/hide crosshair.
- Right-click: menu (toggle / open settings / quit).
- Closing the settings window does NOT quit. Trays **Quit** exits.

## 7. Legacy migration
First run auto-migrates legacy CrossHairLIN configs from `%APPDATA%/CrosshairApp/` (full field mapping); the old folder is kept.

## 8. FAQ
- Crosshair missing? Check tray icon still exists → double-click to re-show; check the log.
- Covered in the game? Use **borderless windowed** mode. Exclusive fullscreen (D3D) can never be overlaid by any overlay tool.
- Gamepad unresponsive? XInput mode required; PS pads via DS4Windows; lower the threshold.

---

# Architecture

```
Main thread   egui settings window (required on main thread since v1.0.1)
Msg thread    Win32 message pump (tray / hotkeys / RawInput / foreground / gamepad events)
Render thread D2D overlay (event-driven, blocks when idle)
Gamepad thread XInput polling (250 Hz)
```

```
src/
├── main.rs        # entry + message-thread wiring
├── config.rs      # preset schema, atomic JSON, legacy migration, hotkey parser
├── overlay/       # D2D renderer (14 shapes, quad colors, outline, rotation, spread, images)
├── input/         # XInput gamepad + RawInput mouse
├── system/        # tray / hotkey / foreground detect / monitors / autostart
├── ui/            # egui settings window (preview, i18n, themes, presets)
└── state.rs       # ADS state machine, preset cycling
```

## Requirements
- Windows 10 / 11 (x64). No runtime dependencies.

## Roadmap
- [x] S0–S5 complete (see README_CN for details)
- [x] **v1.0.1** fix: settings window not showing (egui moved to main thread)
- [ ] **v1.1**: tray "open settings" re-launch, autostart UI toggle, game-binding editor, image file picker

## License
MIT. Original author: 林晓CCC — Bilibili: https://space.bilibili.com/622769073
