# Crosshair Overlay

A Windows desktop crosshair overlay tool built with PySide6. It draws a custom crosshair on top of your game window with click-through support, so it doesn't interfere with gameplay.

Mainly for games without a built-in crosshair, or for players who want to customize the crosshair style. Works best with borderless windowed mode — exclusive fullscreen may not be supported.

[中文说明 (Chinese README)](README_CN.md)

## Features

- **8 crosshair shapes**: cross, dot, square, circle, hollow cross, hollow square, hollow cross + dot, custom image
- **Click-through**: the overlay window doesn't capture mouse events, so the game works normally
- **Drag positioning**: switch to drag mode and move the crosshair anywhere; the position is remembered
- **Customizable parameters**: size, thickness, opacity, color — hollow cross also has gap and line length
- **Border/outline**: add an outline around the crosshair for better visibility on light backgrounds
- **Custom image**: import your own crosshair image with scale and opacity control
- **Preset management**: save multiple configs and switch between them; stored in `%APPDATA%/CrosshairApp/`
- **Global hotkey**: register a global hotkey to toggle crosshair visibility in-game
- **Global right-click toggle** (optional): uses a low-level mouse hook so right-clicking anywhere toggles the crosshair. Note: this intercepts the global right-click, which is disruptive for normal use — enable only when needed
- **Auto topmost on fullscreen** (optional): when the foreground switches to a fullscreen window (e.g. a borderless-fullscreen game), the crosshair is automatically brought back to the top via `SetWindowPos`, so you don't have to reset it manually. Note: does not work for D3D exclusive fullscreen (that's hardware-level display takeover)
- **System tray**: can minimize to the tray and run in the background
- **Bilingual UI**: Chinese / English

## Requirements

- Windows 10 / 11
- Python 3.8+ (to run from source)
- PySide6 6.4+

## Installation

```bash
# Clone the repo
git clone https://github.com/liuroland55/CrossHairLIN.git
cd CrossHairLIN

# Install dependencies
pip install -r requirements.txt

# Run
python crosshair_pyside6.py
```

Don't want to install Python? Download the prebuilt exe from [Releases](https://github.com/liuroland55/CrossHairLIN/releases).

## Usage

1. When you launch the app, the crosshair appears at the center of the screen automatically
2. Pick a shape and tweak parameters on the left — the crosshair updates in real time
3. To move it, click "Drag Mode", drag the crosshair into place, then click back to "Normal Mode" — the position is saved automatically
4. Remember to click "Save Current Config", or save it as a new preset
5. To toggle visibility in-game, set a global hotkey

## Project Structure

```
Crosshair/
├── crosshair_pyside6.py        # Entry point
├── config_ui_pyside6.py        # Main UI (config window)
├── overlay_window_pyside6.py   # Overlay (crosshair rendering + mouse hook)
├── border_settings_dialog.py   # Border settings dialog
├── build_v3.1.0.py             # PyInstaller build script (with UPX)
├── requirements.txt            # Dependencies
├── FAV/                        # App icon assets (favicon_1024.png, app.ico, ...)
├── 小林の准星.exe              # Prebuilt executable (v3.1.0, built via build_v3.1.0.py)
└── 小林の准星V2.00/            # v2.0.0 release package (legacy, exe named 准星程序.exe)
    ├── 准星程序.exe
    └── 版本说明_v2.0.0.txt
```

## How It Works

Two key pieces:

**1. Click-through**: a combination of PySide6 window flags — frameless, stay-on-top, tool window, plus `Qt.WindowTransparentForInput`. The window is visible but doesn't receive mouse events, so clicks pass straight through to the game underneath.

**2. Rendering**: a fullscreen transparent window + a QTimer that triggers a repaint every 50ms, drawing the crosshair with QPainter.

Earlier experiments tried Tkinter, PyGame, the Win32 API, and PyQt5 before settling on PySide6 — mainly because its `WindowTransparentForInput` is native and the most reliable for click-through, with good rendering quality.

The global hotkey uses Windows `RegisterHotKey`. The global right-click uses a low-level mouse hook (`WH_MOUSE_LL`) installed via `SetWindowsHookEx`. Auto topmost on fullscreen uses `SetWindowPos` with `HWND_TOPMOST`.

## Config Files

Configs are stored under `%APPDATA%/CrosshairApp/`, one JSON file per preset. The default preset is `default.json`. You can create, load, save, and delete presets in the UI, and you can also change the storage directory.

## Build

```bash
python build_v3.1.0.py
```

This script packages the app into a single-file exe with PyInstaller, then compresses it with UPX (UPX is auto-downloaded if missing). The output is a release folder and a zip.

## Version History

### v3.1.0
- New: "Hold right-click to toggle" mode — alongside the existing click-to-toggle, you can now choose "Hold to show" or "Hold to hide"
- Removed the popup that used to appear when enabling the right-click toggle option
- Fixed a real crash: holding right-click in "Hold to show" mode could crash the app (the mouse hook was being reinstalled on every press while the button was held, triggering a runaway loop)
- Hollow cross: removed the separate Line Length / Line Thickness sliders — hollow cross shapes now use the main Size / Thickness sliders instead
- More vertical spacing between the hollow-cross sliders so they're not cramped together
- Thickness slider now supports decimals (0.1 increments)
- Fixed: typing a value directly into the entry box next to a slider (Size, Thickness, Opacity, Image Scale, Center Gap, Center Dot Size) didn't apply — it now commits on Enter/blur, with range clamping and invalid-input recovery
- Fixed missing English translations for "Hollow Cross Settings", "Center Dot Size", and "Center Gap"
- Fixed several labels (Center Dot Size, Background Opacity, Select/Clear Background buttons) not refreshing when switching language
- Fixed deleting a preset occasionally showing a false "delete failed" error even though it succeeded
- Fixed the build script's UPX compression step silently failing (WinError 2) due to a relative-path issue

### v3.0.0
- **Control opacity slider**: a new slider next to "Background Opacity" adjusts the background transparency of all cards / input fields / buttons at once, while the text stays fully opaque
- Background-image crop selection no longer shrinks when dragged to the image edge — it keeps its size instead
- Shape dropdown options are now localized (they follow the language instead of showing raw variable names)
- Completed English localization for the Hotkey, Position, Save Current Config, and Theme labels
- App branding unified: Chinese title/window title is **小林の准星**, English title/window title is **Roland's Crosshair**
- App icon now uses the high-resolution icon from `FAV/favicon_1024.png`
- Fixed: switching presets didn't re-apply that preset's hotkey until you re-set it manually
- The app now remembers the last-used preset and language and restores them on next launch (previously always reopened on the default preset, silently dropping any hotkey saved in another preset)

### v2.0.0 (2026-01-18)
- Global right-click toggle for the crosshair (low-level mouse hook)
- Clear-hotkey button
- Full English translation — main UI and border settings dialog both support CN/EN switching
- Fixed global-hook memory management and crash-on-exit issues
- Crosshair can be toggled even when the app is out of focus
- Auto topmost on fullscreen

### v1.1.1 (2025-12-17)
- Crosshair size range extended to 1–100px (was 5px minimum)
- Rendering optimization, lower CPU usage
- Fixed slider precision and config export issues

### v1.1.0 (2025-12-17)
- Drag positioning system — drag the crosshair anywhere
- Position memory — survives hide/show
- Dynamic window-flag toggling for click-through vs. draggable

### v1.0.1 (2025-12-16)
- First release: 8 crosshair shapes + config system + bilingual UI

## Author

**林晓CCC** — Bilibili: https://space.bilibili.com/622769073

MIT License.