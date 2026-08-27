//! Direct2D 覆盖层：事件驱动的分层窗口 + 准星渲染。
//!
//! 架构：
//! - 独立渲染线程，`crossbeam` 通道接收控制消息（显隐/更新/退出）；
//! - 只绘制准星包围的小窗口（非全屏），静止时线程阻塞在通道上（≈0% CPU）；
//! - 动态扩散动画由本线程内部状态机驱动（按预设恢复速度衰减），不占主线程；
//! - 渲染链路：D2D DCRenderTarget → 32bpp DIB → `UpdateLayeredWindow`
//!   （每帧只提交小窗口区域，交给 DWM 合成）。
//!
//! windows-rs 0.58 注意点（实测）：
//! - 可选句柄参数用 `None`（推断为 `Option<&T>`），不要用 `Some(value)`；
//! - `Error::from_win32()` 无参数（取线程最后错误码），自定义码用 `Error::from(WIN32_ERROR)`；
//! - `EndDraw(None, None)`、`DrawBitmap` 只有 5 个参数。

pub mod shapes;

use std::collections::HashMap;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded, after, select};
use log::{info, warn};
use windows::core::{Interface, PCWSTR, w};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{
    COLORREF, ERROR_INVALID_HANDLE, ERROR_INVALID_PARAMETER, HANDLE, HINSTANCE, HWND, POINT,
    RECT, SIZE,
};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED,
    D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F, D2D_SIZE_U, ID2D1SimplifiedGeometrySink,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DEBUG_LEVEL_NONE,
    D2D1_ELLIPSE, D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE, ID2D1Bitmap, ID2D1DCRenderTarget, ID2D1Factory,
    ID2D1PathGeometry, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, AC_SRC_ALPHA,
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, DIB_RGB_COLORS, HBRUSH, HBITMAP, HDC,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetSystemMetrics,
    HCURSOR, HICON, HMENU, PeekMessageW, RegisterClassW, SetWindowPos, ShowWindow,
    TranslateMessage, UpdateLayeredWindow, WNDCLASSW, WNDCLASS_STYLES, HWND_TOPMOST, MSG,
    PM_REMOVE, SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WINDOW_EX_STYLE, WINDOW_STYLE,
};

use crate::config::{OutlineConfig, Preset, QuadColors, Shape};
use crate::overlay::shapes::{
    rotation_safe_radius, Point as GeoPoint, Prim, ShapeGeom, ShapeParams, SLOT_BOTTOM, SLOT_LEFT,
    SLOT_MAIN, SLOT_RIGHT, SLOT_TOP,
};

const WINDOW_CLASS: PCWSTR = w!("ACAJACrosshairOverlay");
const MAX_CANVAS: u32 = 1024;
const MIN_CANVAS: u32 = 16;
const CANVAS_MARGIN: f32 = 8.0;
const ANIM_STEP: Duration = Duration::from_millis(16);

// ===========================================================================
// 对外接口
// ===========================================================================

/// 控制消息
pub enum Msg {
    Update { preset: Arc<Preset>, pos: (i32, i32), visible: bool, expand: f32 },
    Move { pos: (i32, i32) },
    Close,
}

/// 覆盖层句柄（可跨线程克隆发送）
#[derive(Clone)]
pub struct OverlayHandle {
    tx: Sender<Msg>,
}

impl OverlayHandle {
    pub fn update(&self, preset: Arc<Preset>, pos: (i32, i32), visible: bool) {
        let _ = self.tx.send(Msg::Update { preset, pos, visible, expand: 0.0 });
    }

    pub fn update_with_expand(&self, preset: Arc<Preset>, pos: (i32, i32), visible: bool, expand: f32) {
        let _ = self.tx.send(Msg::Update { preset, pos, visible, expand });
    }

    pub fn move_to(&self, pos: (i32, i32)) {
        let _ = self.tx.send(Msg::Move { pos });
    }

    pub fn close(&self) {
        let _ = self.tx.send(Msg::Close);
    }
}

/// 启动覆盖层渲染线程
pub fn start() -> (OverlayHandle, JoinHandle<()>) {
    let (tx, rx) = unbounded();
    let handle = OverlayHandle { tx };
    let thread = std::thread::Builder::new()
        .name("acaja-overlay".into())
        .spawn(move || overlay_thread(rx))
        .expect("spawn overlay thread");
    (handle, thread)
}

// ===========================================================================
// 渲染线程
// ===========================================================================

struct Canvas {
    hwnd: HWND,
    factory: ID2D1Factory,
    dc_target: ID2D1DCRenderTarget,
    hdc: HDC,
    hbmp: HBITMAP,
    #[allow(dead_code)]
    bits: *mut u8,
    w: u32,
    h: u32,
    brushes: HashMap<u32, ID2D1SolidColorBrush>,
    image: Option<ImageCache>,
    preset: Arc<Preset>,
    pos: (i32, i32),
    visible: bool,
    shown: bool,
    expand: f32,
    animating: bool,
}

struct ImageCache {
    key: String,
    bitmap: ID2D1Bitmap,
    w: u32,
    h: u32,
}

fn overlay_thread(rx: Receiver<Msg>) {
    info!("overlay 线程启动");
    let mut canvas = match create_canvas() {
        Ok(c) => c,
        Err(e) => {
            warn!("覆盖层初始化失败: {e}");
            return;
        }
    };

    loop {
        let mut dirty = false;

        let mut quit = false;
        if canvas.animating {
            // 动画期：16ms 节拍驱动衰减
            let tick = after(ANIM_STEP);
            select! {
                recv(rx) -> msg => {
                    quit = handle_msg(&mut canvas, &mut dirty, msg);
                }
                recv(tick) -> _ => {
                    let rate = 1000.0 / canvas.preset.dynamic.recover_ms.max(1) as f32; // px/s
                    canvas.expand = (canvas.expand - rate * 0.016).max(0.0);
                    canvas.animating = canvas.expand > 0.01;
                    dirty = true;
                }
            }
        } else {
            // 空闲：纯阻塞等消息（≈0 唤醒，最小化 CPU 占用）
            select! {
                recv(rx) -> msg => {
                    quit = handle_msg(&mut canvas, &mut dirty, msg);
                }
            }
        }
        if quit {
            canvas.cleanup();
            info!("overlay 线程退出");
            return;
        }

        pump_messages(canvas.hwnd);

        // 显隐同步
        if canvas.visible && !canvas.shown {
            unsafe { ShowWindow(canvas.hwnd, SW_SHOWNOACTIVATE); }
            canvas.shown = true;
            dirty = true;
        } else if !canvas.visible && canvas.shown {
            unsafe { ShowWindow(canvas.hwnd, SW_HIDE); }
            canvas.shown = false;
        }

        if dirty && canvas.visible {
            if let Err(e) = render_frame(&mut canvas) {
                warn!("渲染失败: {e}");
            }
        }
    }
}


fn handle_msg(canvas: &mut Canvas, dirty: &mut bool, msg: Result<Msg, crossbeam_channel::RecvError>) -> bool {
    match msg {
        Ok(m) => match m {
            Msg::Update { preset, pos, visible, expand } => {
                canvas.preset = preset;
                canvas.pos = pos;
                canvas.visible = visible;
                canvas.expand = expand;
                canvas.animating = expand > 0.01 && canvas.preset.dynamic.fire_expand_px > 0.0;
                *dirty = true;
            }
            Msg::Move { pos } => {
                canvas.pos = pos;
                *dirty = true;
            }
            Msg::Close => return true,
        },
        Err(_) => {}
    }
    false
}

fn pump_messages(hwnd: HWND) {
    unsafe {
        let mut msg = MSG::default();
        // 注意：传 None（Option<&HWND>），窗口气泡消息全部在这个线程
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = hwnd;
    }
}

// ===========================================================================
// 渲染
// ===========================================================================

fn render_frame(canvas: &mut Canvas) -> Result<(), windows::core::Error> {
    let preset = canvas.preset.clone();
    let params = ShapeParams {
        size: preset.size,
        thickness: preset.thickness,
        gap: preset.hollow.gap,
        dot: preset.hollow.center_dot_size,
        expand: canvas.expand,
        outline: if preset.outline.enabled { preset.outline.thickness } else { 0.0 },
    };

    // ---- 画布尺寸（旋转安全半径） ----
    let need_side = match preset.shape {
        Shape::CustomImage => {
            let w = preset.image.scale * 64.0; // 未加载时占位
            (w.max(16.0).ceil() as u32).clamp(MIN_CANVAS, MAX_CANVAS)
        }
        _ => {
            let geom = shapes::build(preset.shape, &params)
                .ok_or_else(|| windows::core::Error::from(ERROR_INVALID_PARAMETER))?;
            let r = rotation_safe_radius(&geom) + CANVAS_MARGIN;
            (r * 2.0).ceil().clamp(MIN_CANVAS as f32, MAX_CANVAS as f32) as u32
        }
    };
    if canvas.w != need_side || canvas.h != need_side {
        ensure_canvas_size(canvas, need_side, need_side)?;
    }

    unsafe {
        let rect = RECT { left: 0, top: 0, right: canvas.w as i32, bottom: canvas.h as i32 };
        canvas.dc_target.BindDC(canvas.hdc, &rect)?;

        let rt: ID2D1RenderTarget = canvas.dc_target.cast()?;
        let _ = rt.BeginDraw();
        let clear = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        rt.Clear(Some(&clear as *const D2D1_COLOR_F));

        let cx = canvas.w as f32 / 2.0;
        let cy = canvas.h as f32 / 2.0;
        let theta = preset.rotation.to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        // 行向量约定：M = Translate ∘ Rotate
        let transform = Matrix3x2 {
            M11: cos_t, M12: sin_t,
            M21: -sin_t, M22: cos_t,
            M31: cx, M32: cy,
        };
        rt.SetTransform(&transform);

        // ---- 矢量形状（描边层 → 主层） ----
        if let Some(geom) = shapes::build(preset.shape, &params) {
            let outline = if preset.outline.enabled { Some(&preset.outline) } else { None };
            draw_prims(canvas, &rt, &geom, &preset, outline, true)?;
            draw_prims(canvas, &rt, &geom, &preset, outline, false)?;
        }

        // ---- 自定义图片 ----
        if preset.shape == Shape::CustomImage {
            draw_custom_image(canvas, &rt)?;
        }

        // ---- 后坐力恢复指示条 ----
        if preset.dynamic.recoil_indicator && canvas.expand > 0.2 {
            let y = cy + preset.size * 0.5 + 14.0 + canvas.expand * 2.0;
            let half = canvas.expand * 3.0 + 4.0;
            let b = brush(canvas, &rt, (1.0, 1.0, 1.0), 0.6)?;
            let _ = rt.DrawLine(
                D2D_POINT_2F { x: cx - half, y },
                D2D_POINT_2F { x: cx + half, y },
                &b, 2.0, None,
            );
        }

        let _ = rt.EndDraw(None, None);
    }

    // ---- 提交到屏幕 ----
    let (left, top, w, h) = {
        let w = canvas.w as i32;
        let h = canvas.h as i32;
        (canvas.pos.0 - w / 2, canvas.pos.1 - h / 2, w, h)
    };
    let blend = BLENDFUNCTION {
        BlendOp: 0, // AC_SRC_OVER
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        let size = SIZE { cx: w, cy: h };
        let dst = POINT { x: left, y: top };
        let src = POINT { x: 0, y: 0 };
        UpdateLayeredWindow(
            canvas.hwnd,
            None,
            Some(&dst),
            Some(&size),
            canvas.hdc,
            Some(&src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_prims(
    canvas: &mut Canvas,
    rt: &ID2D1RenderTarget,
    geom: &ShapeGeom,
    preset: &Preset,
    outline: Option<&OutlineConfig>,
    outline_pass: bool,
) -> Result<(), windows::core::Error> {
    let colors = preset.active_colors();
    let main_color = parse_hex(&preset.color);
    let use_outline = outline_pass && outline.map(|o| o.enabled).unwrap_or(false);
    let outline = if use_outline { outline } else { None };
    let delta = outline.map(|o| o.thickness).unwrap_or(0.0);
    let stroke_w = if use_outline { preset.thickness + delta * 2.0 } else { preset.thickness };
    let op = if use_outline { outline.unwrap().opacity } else { preset.opacity };

    unsafe {
        for prim in &geom.prims {
            let slot = match prim {
                Prim::Line { slot, .. }
                | Prim::RectFill { slot, .. }
                | Prim::RectStroke { slot, .. }
                | Prim::Dot { slot, .. }
                | Prim::Ring { slot, .. }
                | Prim::PolyFill { slot, .. } => *slot,
            };
            let (r, g, b) = if use_outline {
                parse_hex(&outline.unwrap().color)
            } else {
                slot_color(&colors, main_color, slot)
            };
            let b = brush(canvas, rt, (r, g, b), op)?;

            match prim {
                Prim::Line { x1, y1, x2, y2, .. } => {
                    let _ = rt.DrawLine(
                        D2D_POINT_2F { x: *x1, y: *y1 },
                        D2D_POINT_2F { x: *x2, y: *y2 },
                        &b, stroke_w, None,
                    );
                }
                Prim::RectFill { cx, cy, w, h, .. } => {
                    let r = D2D_RECT_F {
                        left: cx - w / 2.0 - delta,
                        top: cy - h / 2.0 - delta,
                        right: cx + w / 2.0 + delta,
                        bottom: cy + h / 2.0 + delta,
                    };
                    let _ = rt.FillRectangle(&r, &b);
                }
                Prim::RectStroke { cx, cy, w, h, .. } => {
                    let r = D2D_RECT_F {
                        left: cx - w / 2.0 - delta,
                        top: cy - h / 2.0 - delta,
                        right: cx + w / 2.0 + delta,
                        bottom: cy + h / 2.0 + delta,
                    };
                    let sw = if use_outline { stroke_w + delta } else { stroke_w };
                    let _ = rt.DrawRectangle(&r, &b, sw, None);
                }
                Prim::Dot { cx, cy, r, .. } => {
                    let e = D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: *cx, y: *cy },
                        radiusX: r + delta,
                        radiusY: r + delta,
                    };
                    let _ = rt.FillEllipse(&e, &b);
                }
                Prim::Ring { cx, cy, r, .. } => {
                    let e = D2D1_ELLIPSE {
                        point: D2D_POINT_2F { x: *cx, y: *cy },
                        radiusX: r + delta,
                        radiusY: r + delta,
                    };
                    let sw = if use_outline { stroke_w + delta } else { stroke_w };
                    let _ = rt.DrawEllipse(&e, &b, sw, None);
                }
                Prim::PolyFill { pts, .. } => {
                    if use_outline {
                        // 描边层：沿多边形边画粗线
                        for i in 0..pts.len() {
                            let a = pts[i];
                            let c = pts[(i + 1) % pts.len()];
                            let _ = rt.DrawLine(
                                D2D_POINT_2F { x: a.x, y: a.y },
                                D2D_POINT_2F { x: c.x, y: c.y },
                                &b, stroke_w + delta, None,
                            );
                        }
                    } else {
                        let path = fill_polygon(&canvas.factory, pts)?;
                        let _ = rt.FillGeometry(&path, &b, None);
                    }
                }
            }
        }
    }
    Ok(())
}

/// 构建多边形填充几何
fn fill_polygon(
    factory: &ID2D1Factory,
    pts: &[GeoPoint],
) -> Result<ID2D1PathGeometry, windows::core::Error> {
    let path = unsafe { factory.CreatePathGeometry()? };
    let sink = unsafe { path.Open()? };
    let sink: ID2D1SimplifiedGeometrySink = sink.cast()?;
    unsafe {
        sink.BeginFigure(D2D_POINT_2F { x: pts[0].x, y: pts[0].y }, D2D1_FIGURE_BEGIN_FILLED);
        let points: Vec<D2D_POINT_2F> = pts.iter().map(|p| D2D_POINT_2F { x: p.x, y: p.y }).collect();
        sink.AddLines(&points);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
    }
    Ok(path)
}

fn draw_custom_image(canvas: &mut Canvas, rt: &ID2D1RenderTarget) -> Result<(), windows::core::Error> {
    let preset = &canvas.preset;
    if preset.image.path.is_empty() {
        return Ok(());
    }
    let key = format!("{}@{:.4}", preset.image.path, preset.image.scale);
    let need_load = match &canvas.image {
        Some(c) => c.key != key,
        None => true,
    };
    if need_load {
        match load_image_bitmap(rt, &preset.image.path, preset.image.scale) {
            Ok((bitmap, w, h)) => canvas.image = Some(ImageCache { key, bitmap, w, h }),
            Err(e) => {
                canvas.image = None;
                warn!("加载自定义图片失败 {}: {e}", preset.image.path);
                return Ok(());
            }
        }
    }
    if let Some(img) = &canvas.image {
        let cx = canvas.w as f32 / 2.0;
        let cy = canvas.h as f32 / 2.0;
        let dst = D2D_RECT_F {
            left: cx - img.w as f32 / 2.0,
            top: cy - img.h as f32 / 2.0,
            right: cx + img.w as f32 / 2.0,
            bottom: cy + img.h as f32 / 2.0,
        };
        unsafe {
            let _ = rt.DrawBitmap(
                &img.bitmap,
                Some(&dst),
                preset.opacity,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }
    }
    Ok(())
}

fn brush(
    canvas: &mut Canvas,
    rt: &ID2D1RenderTarget,
    (r, g, b): (f32, f32, f32),
    a: f32,
) -> Result<ID2D1SolidColorBrush, windows::core::Error> {
    let key = ((r * 255.0) as u32 & 0xFF) << 24
        | ((g * 255.0) as u32 & 0xFF) << 16
        | ((b * 255.0) as u32 & 0xFF) << 8
        | (a * 255.0) as u32 & 0xFF;
    if let Some(b) = canvas.brushes.get(&key) {
        return Ok(b.clone());
    }
    let color = D2D1_COLOR_F { r, g, b, a };
    let b = unsafe { rt.CreateSolidColorBrush(&color, None)? };
    canvas.brushes.insert(key, b.clone());
    Ok(b)
}

fn slot_color(colors: &QuadColors, main: (f32, f32, f32), slot: u8) -> (f32, f32, f32) {
    match slot {
        SLOT_TOP => parse_hex(&colors.top),
        SLOT_BOTTOM => parse_hex(&colors.bottom),
        SLOT_LEFT => parse_hex(&colors.left),
        SLOT_RIGHT => parse_hex(&colors.right),
        SLOT_MAIN => main,
        _ => main,
    }
}

/// 供 UI 预览复用的取色（与屏幕渲染严格一致）
pub(crate) fn slot_color_pub(colors: &QuadColors, main: (f32, f32, f32), slot: u8) -> (f32, f32, f32) {
    slot_color(colors, main, slot)
}

/// 解析 "#RRGGBB" → (r, g, b) 0.0-1.0；失败返回红色
pub fn parse_hex(s: &str) -> (f32, f32, f32) {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return (1.0, 0.0, 0.0);
    }
    match u32::from_str_radix(s, 16) {
        Ok(v) => (
            ((v >> 16) & 0xFF) as f32 / 255.0,
            ((v >> 8) & 0xFF) as f32 / 255.0,
            (v & 0xFF) as f32 / 255.0,
        ),
        Err(_) => (1.0, 0.0, 0.0),
    }
}

/// 图片 → 预乘 BGRA → D2D 位图（只解码一次并缓存）
fn load_image_bitmap(
    rt: &ID2D1RenderTarget,
    path: &str,
    scale: f32,
) -> std::io::Result<(ID2D1Bitmap, u32, u32)> {
    let img = image::open(path).map_err(|e| std::io::Error::other(e.to_string()))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let nw = ((w as f32 * scale).round() as u32).max(1).min(1024);
    let nh = ((h as f32 * scale).round() as u32).max(1).min(1024);
    let scaled = image::imageops::resize(&rgba, nw, nh, image::imageops::FilterType::Triangle);

    let mut data = Vec::with_capacity((nw * nh * 4) as usize);
    for p in scaled.pixels() {
        let (r, g, b, a) = (p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32);
        data.push((b * a / 255) as u8);
        data.push((g * a / 255) as u8);
        data.push((r * a / 255) as u8);
        data.push(a as u8);
    }

    let size = D2D_SIZE_U { width: nw, height: nh };
    let props = windows::Win32::Graphics::Direct2D::D2D1_BITMAP_PROPERTIES {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
    };
    let bitmap = unsafe {
        rt.CreateBitmap(size, Some(data.as_ptr() as *const std::ffi::c_void), nw * 4, &props)?
    };
    Ok((bitmap, nw, nh))
}

// ===========================================================================
// 画布与窗口生命周期
// ===========================================================================

fn create_canvas() -> Result<Canvas, windows::core::Error> {
    unsafe {
        let factory: ID2D1Factory = D2D1CreateFactory(
            D2D1_FACTORY_TYPE_SINGLE_THREADED,
            Some(&D2D1_FACTORY_OPTIONS { debugLevel: D2D1_DEBUG_LEVEL_NONE }),
        )?;

        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 0.0,
            dpiY: 0.0,
            usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let dc_target: ID2D1DCRenderTarget = factory.CreateDCRenderTarget(&props)?;

        // ---- 覆盖窗口 ----
        let class_name = WINDOW_CLASS;
        let wc = WNDCLASSW {
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(overlay_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: HINSTANCE(GetModuleHandleW(PCWSTR::null())?.0),
            hIcon: HICON(std::ptr::null_mut()),
            hCursor: HCURSOR(std::ptr::null_mut()),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszMenuName: PCWSTR::null(),
            lpszClassName: class_name,
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(
                WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOPMOST.0,
            ),
            class_name,
            w!("ACAJA Overlay"),
            WINDOW_STYLE(WS_POPUP.0),
            0, 0, MIN_CANVAS as i32, MIN_CANVAS as i32,
            None, None, None, None,
        )?;

        // 初始：置顶 + 不激活（HWND_TOPMOST 直接传，不要 Some）
        SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE).ok();

        let (hdc, hbmp, bits, w, h) = create_dib(MIN_CANVAS, MIN_CANVAS)?;

        Ok(Canvas {
            hwnd,
            factory,
            dc_target,
            hdc,
            hbmp,
            bits,
            w,
            h,
            brushes: HashMap::new(),
            image: None,
            preset: Arc::new(Preset::default()),
            pos: (0, 0),
            visible: false,
            shown: false,
            expand: 0.0,
            animating: false,
        })
    }
}

/// 32bpp 顶向下 DIB + 内存 DC
unsafe fn create_dib(
    w: u32,
    h: u32,
) -> Result<(HDC, HBITMAP, *mut u8, u32, u32), windows::core::Error> {
    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w as i32;
    bmi.bmiHeader.biHeight = -(h as i32); // 顶向下
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let hbmp = CreateDIBSection(
        HDC(std::ptr::null_mut()),
        &bmi,
        DIB_RGB_COLORS,
        &mut bits,
        HANDLE(std::ptr::null_mut()),
        0,
    )?;

    let hdc = CreateCompatibleDC(None);
    if hdc.is_invalid() {
        let _ = DeleteObject(hbmp);
        return Err(windows::core::Error::from(ERROR_INVALID_HANDLE));
    }
    SelectObject(hdc, hbmp);
    Ok((hdc, hbmp, bits as *mut u8, w, h))
}

fn ensure_canvas_size(
    canvas: &mut Canvas,
    w: u32,
    h: u32,
) -> Result<(), windows::core::Error> {
    unsafe {
        if !canvas.hbmp.is_invalid() {
            let _ = DeleteObject(canvas.hbmp);
        }
        if !canvas.hdc.is_invalid() {
            let _ = DeleteDC(canvas.hdc);
        }
        let (hdc, hbmp, bits, nw, nh) = create_dib(
            w.max(MIN_CANVAS).min(MAX_CANVAS),
            h.max(MIN_CANVAS).min(MAX_CANVAS),
        )?;
        canvas.hdc = hdc;
        canvas.hbmp = hbmp;
        canvas.bits = bits;
        canvas.w = nw;
        canvas.h = nh;
        canvas.brushes.clear();
        canvas.image = None;
    }
    Ok(())
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

impl Canvas {
    fn cleanup(&mut self) {
        unsafe {
            if !self.hdc.is_invalid() {
                let _ = DeleteDC(self.hdc);
            }
            if !self.hbmp.is_invalid() {
                let _ = DeleteObject(self.hbmp);
            }
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// 主屏中心（S3 起接入多显示器/前台窗口）
pub fn primary_screen_center() -> (i32, i32) {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        (w / 2, h / 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_works() {
        assert_eq!(parse_hex("#FF0000"), (1.0, 0.0, 0.0));
        assert_eq!(parse_hex("#00FF00"), (0.0, 1.0, 0.0));
        assert_eq!(parse_hex("0000ff"), (0.0, 0.0, 1.0));
        // 非法 → 红
        assert_eq!(parse_hex("nope"), (1.0, 0.0, 0.0));
    }

    #[test]
    fn quad_slot_colors() {
        let q = QuadColors {
            top: "#00FF00".into(),
            bottom: "#00FFFF".into(),
            left: "#0000FF".into(),
            right: "#FF00FF".into(),
        };
        assert_eq!(slot_color(&q, (1.0, 0.0, 0.0), SLOT_TOP), (0.0, 1.0, 0.0));
        assert_eq!(slot_color(&q, (1.0, 0.0, 0.0), SLOT_BOTTOM), (0.0, 1.0, 1.0));
        assert_eq!(slot_color(&q, (1.0, 0.0, 0.0), SLOT_LEFT), (0.0, 0.0, 1.0));
        assert_eq!(slot_color(&q, (1.0, 0.0, 0.0), SLOT_RIGHT), (1.0, 0.0, 1.0));
        assert_eq!(slot_color(&q, (1.0, 0.0, 0.0), SLOT_MAIN), (1.0, 0.0, 0.0));
    }
}