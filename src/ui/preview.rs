//! 准星实时预览（egui 绘制，与屏幕 D2D 渲染共用同一套几何）。
//!
//! 棋盘格背景模拟过场画面，准星按预设参数居中渲染，
//! 描边层 + 四象限分色与 overlay 的 draw_prims 逻辑镜像。
//!
//! v1.3：棋盘格改成"深空玻璃屏"语言——深黑灰格 + 镜面内边 + 顶部高光，随主题切换；
//! 三个入口共用一份实现，只是格子大小 / 扩散量不同。

use egui::{pos2, Color32, Mesh, Rect, Shape, Stroke, Ui};

use crate::config::Preset;
use crate::overlay::parse_hex;
use crate::overlay::shapes::{rotation_safe_radius, Prim, ShapeParams, SLOT_BOTTOM, SLOT_LEFT, SLOT_RIGHT, SLOT_TOP};

/// 在给定区域绘制预览（`alpha` 是整体淡入系数：卡片入场时预览跟着一起渐显）
pub fn paint_preview(ui: &mut Ui, rect: Rect, preset: &Preset, lang: crate::i18n::Lang, alpha: f32) {
    paint_preview_impl(ui, rect, preset, lang, 8.0, 0.0, alpha);
}

/// 同 `paint_preview`，可指定棋盘格边长。
///
/// 模板缩略图（8 个格子同时绘制）用更大的格子，配合 Mesh 合并，
/// 让整帧的图元数量保持在很低的水位（拖动滑杆时每帧都会重绘整个预览区）。
pub fn paint_preview_cells(ui: &mut Ui, rect: Rect, preset: &Preset, lang: crate::i18n::Lang, cell: f32, alpha: f32) {
    paint_preview_impl(ui, rect, preset, lang, cell, 0.0, alpha);
}

/// 同 `paint_preview`，但把「开火扩散」量 `expand` 叠加到几何上。
///
/// 动态准星分区用它演示"开火瞬间"的形态（与 overlay 的 `ShapeParams.expand` 是同一条路径）。
pub fn paint_preview_expanded(ui: &mut Ui, rect: Rect, preset: &Preset, lang: crate::i18n::Lang, expand: f32, alpha: f32) {
    paint_preview_impl(ui, rect, preset, lang, 8.0, expand, alpha);
}

/// 预览实现：`cell` 棋盘格边长，`expand` 叠加的开火扩散量（0 = 静止形态），`alpha` 整体淡入系数
fn paint_preview_impl(
    ui: &mut Ui,
    rect: Rect,
    preset: &Preset,
    lang: crate::i18n::Lang,
    cell: f32,
    expand: f32,
    alpha: f32,
) {
    let k = alpha.clamp(0.0, 1.0);
    if k <= 0.004 {
        return;
    }
    let painter = ui.painter_at(rect);
    let dark = ui.visuals().dark_mode;
    // 整块预览跟着卡片一起淡入（否则入场时会"啪"地整块跳出来）
    let fa = |c: Color32| -> Color32 {
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * k).round().clamp(0.0, 255.0) as u8)
    };

    // ---- 棋盘格背景（单个 Mesh：每格 2 个三角形，一次提交） ----
    let cell = cell.max(4.0);
    // 深色：深邃黑灰（#18181A / #111112），不要纯黑；浅色：雾灰
    let (c_light, c_dark, c_edge) = if dark {
        (fa(Color32::from_gray(24)), fa(Color32::from_gray(17)), fa(Color32::from_gray(38)))
    } else {
        (fa(Color32::from_gray(228)), fa(Color32::from_gray(214)), fa(Color32::from_gray(182)))
    };
    // 3px 边框：既像「屏幕边框」，也把方角棋盘与圆角外框的差值藏起来
    painter.rect_filled(rect, 10.0, c_edge);
    let grid = rect.shrink(3.0);
    let mut mesh = Mesh::default();
    let cols = (grid.width() / cell).ceil().max(1.0) as i32;
    let rows = (grid.height() / cell).ceil().max(1.0) as i32;
    for row in 0..rows {
        for col in 0..cols {
            let x0 = grid.left() + col as f32 * cell;
            let y0 = grid.top() + row as f32 * cell;
            let x1 = (x0 + cell).min(grid.right());
            let y1 = (y0 + cell).min(grid.bottom());
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let c = if (row + col) % 2 == 0 { c_light } else { c_dark };
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(pos2(x0, y0), c);
            mesh.colored_vertex(pos2(x1, y0), c);
            mesh.colored_vertex(pos2(x0, y1), c);
            mesh.colored_vertex(pos2(x1, y1), c);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 2, base + 1, base + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
    // 玻璃屏：外框 + 顶部一道镜面高光（与卡片的玻璃语言一致）
    painter.rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0_f32, fa(if dark { Color32::from_white_alpha(24) } else { Color32::from_black_alpha(30) })),
    );
    painter.line_segment(
        [
            pos2(rect.left() + 14.0, rect.top() + 3.5),
            pos2(rect.right() - 14.0, rect.top() + 3.5),
        ],
        Stroke::new(1.0_f32, fa(if dark { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(150) })),
    );

    // ---- 几何 ----
    let params = ShapeParams::from_preset(preset, expand);
    let Some(geom) = crate::overlay::shapes::build(preset.shape, &params) else {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            crate::ui::strings::t(lang, "custom_image"),
            egui::FontId::proportional(13.0),
            fa(if dark { Color32::from_gray(170) } else { Color32::from_gray(110) }),
        );
        return;
    };

    let _radius = rotation_safe_radius(&geom).max(1.0);
    // v1.1.5：固定比例缩放（消除「小尺寸被放大、大尺寸饱和」的非线性）。
    // 以 size=200（最大有效状态）时的几何半径为基准，保证拖动大小始终线性；
    // 预览区半宽 144px 对应几何半径 208px（≈ 实际比例 0.69x）。
    let scale = (rect.width().min(rect.height()) * 0.72) / 208.0;
    let cx = rect.center().x;
    let cy = rect.center().y;
    let theta = preset.rotation.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();
    let tf = |x: f32, y: f32| {
        pos2(
            cx + (x * cos_t - y * sin_t) * scale,
            cy + (x * sin_t + y * cos_t) * scale,
        )
    };

    let colors = preset.active_colors();
    let main = parse_hex(&preset.color);
    let outline_enabled = preset.outline.enabled;
    let outline = &preset.outline;

    let color32 = |(r, g, b): (f32, f32, f32), a: f32| -> Color32 {
        let a255 = (a.clamp(0.0, 1.0) * 255.0 * k) as u8;
        Color32::from_rgba_unmultiplied(
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
            a255,
        )
    };
    let slot_color = |slot: u8| -> (f32, f32, f32) {
        match slot {
            SLOT_TOP => parse_hex(&colors.top),
            SLOT_BOTTOM => parse_hex(&colors.bottom),
            SLOT_LEFT => parse_hex(&colors.left),
            SLOT_RIGHT => parse_hex(&colors.right),
            _ => main,
        }
    };

    for prim in &geom.prims {
        let slot = match prim {
            Prim::Line { slot, .. }
            | Prim::RectFill { slot, .. }
            | Prim::RectStroke { slot, .. }
            | Prim::Dot { slot, .. }
            | Prim::Ring { slot, .. }
            | Prim::PolyFill { slot, .. } => *slot,
        };
        // 描边层
        if outline_enabled {
            let oc = color32(parse_hex(&outline.color), outline.opacity);
            paint_prim(
                ui,
                prim,
                &tf,
                &oc,
                preset.thickness + outline.thickness * 2.0,
                outline.thickness,
                scale,
            );
        }
        // 主层
        let color = color32(slot_color(slot), preset.opacity);
        paint_prim(ui, prim, &tf, &color, preset.thickness, 0.0, scale);
    }
}

/// 绘制单个图元。stroke_w 主线宽；delta > 0 时为描边层（外扩/加粗）。
fn paint_prim(
    ui: &mut Ui,
    prim: &Prim,
    tf: &dyn Fn(f32, f32) -> egui::Pos2,
    color: &Color32,
    stroke_w: f32,
    delta: f32,
    scale: f32,
) {
    let painter = ui.painter();
    let stroke = Stroke::new(stroke_w.max(0.5), *color);
    match prim {
        Prim::Line { x1, y1, x2, y2, .. } => {
            painter.add(Shape::line_segment([tf(*x1, *y1), tf(*x2, *y2)], stroke));
        }
        Prim::RectFill { cx, cy, w, h, .. } => {
            let rect = Rect::from_center_size(
                tf(*cx, *cy),
                egui::vec2((w + delta * 2.0) * scale, (h + delta * 2.0) * scale),
            );
            painter.add(Shape::rect_filled(rect, 0.0, *color));
        }
        Prim::RectStroke { cx, cy, w, h, .. } => {
            let rect = Rect::from_center_size(
                tf(*cx, *cy),
                egui::vec2((w + delta * 2.0) * scale, (h + delta * 2.0) * scale),
            );
            painter.add(Shape::rect_stroke(rect, 0.0, stroke));
        }
        Prim::Dot { cx, cy, r, .. } => {
            painter.add(Shape::circle_filled(tf(*cx, *cy), (r + delta) * scale, *color));
        }
        Prim::Ring { cx, cy, r, .. } => {
            painter.add(Shape::circle_stroke(tf(*cx, *cy), (r + delta) * scale, stroke));
        }
        Prim::PolyFill { pts, .. } => {
            if delta > 0.0 {
                // 描边层：沿边画粗线
                for i in 0..pts.len() {
                    let a = pts[i];
                    let c = pts[(i + 1) % pts.len()];
                    painter.add(Shape::line_segment(
                        [tf(a.x, a.y), tf(c.x, c.y)],
                        Stroke::new(stroke_w + delta, *color),
                    ));
                }
            } else {
                let points: Vec<egui::Pos2> = pts.iter().map(|p| tf(p.x, p.y)).collect();
                painter.add(Shape::convex_polygon(points, *color, Stroke::NONE));
            }
        }
    }
}
