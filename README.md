# ACAJA — Crosshair Overlay for Windows

>[!NOTE]
> **In active development (Milestone S2)**: rendering core and config system are done and CI-verified; system integration (tray/hotkeys), input layer (gamepad ADS), and settings UI are coming per the [Roadmap](#roadmap).

ACAJA is a Windows desktop crosshair overlay tool written in **Rust** (`windows-rs` + Direct2D). It is a complete rewrite of [CrossHairLIN](https://github.com/liuroland55/CrossHairLIN) (小林の准星, Python/PySide6).

It draws a custom crosshair on top of your game window with click-through support, so it doesn't interfere with gameplay. Best for games without a built-in crosshair, or players who want a custom style (works best in borderless windowed mode).

## Key improvements over the Python version

| | Legacy (v3.1.0, PySide6) | ACAJA (Rust) |
|---|---|---|
| Size | 46 MB single exe | **~2.7 MB** single exe |
| Architecture | Fullscreen transparent window + 20 FPS timer repaint | **Event-driven, crosshair-sized window only** (~0% CPU when idle) |
| Rendering | QPainter (CPU) | **Direct2D hardware-accelerated** (GPU composited) |
| Startup | ~1–2 s | milliseconds |
| Click-through | `WindowTransparentForInput` | Native `WS_EX_LAYERED\|TRANSPARENT` layered window |
| Shapes | 8 | **14** (incl. quad-color, dynamic spread, recoil indicator) |
| Runtime deps | Python 3.8+ / PySide6 | none — double-click and run |

## Implemented so far (S0–S2)

- **14 crosshair shapes**: cross, dot, square, circle, hollow cross, hollow square, hollow cross+dot, chevron, triangle, V, T, brackets, gap hair, custom image
- **Multicolor quad crosshair**: independent colors for top/bottom/left/right arms (Crosshair X-style)
- **Outline / rotation / opacity / thickness / gap / center dot** — fully adjustable
- **Dynamic crosshair**: fire spread + recovery decay (recoil recovery indicator)
- **Custom image**: PNG decoded once and cached (scaled), no repeated disk I/O
- **Config system**: per-preset JSON files under `%APPDATA%/ACAJACrosshair/` (atomic writes)
- **One-click migration** from the legacy app: first run automatically converts `%APPDATA%/CrosshairApp/*.json` (shape/size/color/position/hotkey/outline/gamepad fields fully mapped); the old folder is kept untouched
- **Hotkey parsing** compatible with legacy `Ctrl+F1`-style strings (registration lands in S3)
- **Single-instance guard, file logging** at `%APPDATA%/ACAJACrosshair/acaja.log`

## Download & usage

No Python/Rust install required — grab the exe from GitHub Actions:

1. Open the [Actions page](https://github.com/Aacaja/ACAJACrosshair/actions) → latest run → **Artifacts** → download `ACAJA-windows-x64`
2. Unzip and run `ACAJA-v1.0.0-x64.exe`

> You can also trigger a build manually via **workflow_dispatch**, or wait for a tagged [Release](https://github.com/Aacaja/ACAJACrosshair/releases).

## Architecture

```
src/
├── main.rs        # Entry: logging → single-instance guard → config migration → subsystems
├── lib.rs         # Brand constants & app data dir
├── config.rs      # Preset schema, atomic JSON persistence, legacy migration, hotkey parser
├── i18n.rs        # zh/en strings (compile-checked structs)
└── overlay/
    ├── mod.rs     # D2D render thread: layered window + UpdateLayeredWindow
    └── shapes.rs  # Pure geometry for 14 shapes (quad colors, spread, bounds)
```

**Rendering architecture** (fundamentally different from legacy):

- Dedicated render thread driven by a channel (`crossbeam`) of control messages
- Only a **small window covering the crosshair bounding box** (16–1024 px) — never fullscreen
- Thread blocks on the channel when idle (~0% CPU); repaint only on config/position/visibility change
- Pipeline: D2D `DCRenderTarget` → 32bpp premultiplied DIB → `UpdateLayeredWindow` (DWM composited)
- Brushes/images cached; animation decay runs inside the render thread

**Thread model** (full form from S3):

```
Main thread   Win32 message pump (global hotkeys / WinEvent foreground / tray)
Render thread D2D overlay (event-driven, blocks when idle)
Input thread  Gamepad XInput polling (250 Hz) → ADS auto-hide state machine
UI thread     egui settings window (own loop)
```

## Build (GitHub Actions)

Pushes build automatically on `windows-latest`; artifacts are uploaded per run:

```yaml
# .github/workflows/build.yml
on: { push: [main, v* tags], workflow_dispatch }
steps: checkout → rust-toolchain → rust-cache → cargo build --release → cargo test → package → artifact
```

- Pushing a `v*` tag automatically publishes a GitHub Release
- All unit tests (currently 26) run in CI with every build

## Roadmap

- [x] **S0** Scaffold + CI pipeline
- [x] **S1** Config layer: preset schema, atomic persistence, legacy migration, i18n
- [x] **S2** Overlay: D2D rendering, 14 shapes, multicolor, outline, rotation, dynamic spread, custom image
- [ ] **S3** System layer: tray, global hotkeys, per-game auto profiles (foreground exe detection), multi-monitor, window snap, autostart
- [ ] **S4** Input layer: Raw Input fire detection + **gamepad ADS auto-hide** (Apex-style: hold-hide / toggle / hold-show modes, adjustable threshold) + right-click quick toggle
- [ ] **S5** UI: egui bilingual settings window (preview canvas, param panel, game binding table, preset manager, import/export)
- [ ] **S6** Polish: icon set, docs, first v1.0.0 release

## Config directory

```
%APPDATA%/ACAJACrosshair/
├── app.json              # App-level settings (language/theme/autostart/game bindings)
├── presets/default.json  # Presets, one file each
└── acaja.log             # Run log (for troubleshooting)
```

Legacy (`CrosshairApp`) configs are **never deleted**; a failed migration does not affect startup.

## Requirements

- Windows 10 / 11 (x64)
- No runtime dependencies

## Known limitations

- **Exclusive fullscreen (D3D) cannot be overlaid**: that's hardware-level display takeover for any non-injecting overlay; use borderless windowed mode (industry standard).
- Native gamepad support is Xbox-lineage (XInput); DS4/DS5 need DS4Windows-style driver emulation.

## License

MIT.

Original project author: 林晓CCC — Bilibili: https://space.bilibili.com/622769073