//! 准星实时预览（egui 绘制，与屏幕 D2D 渲染共用同一套几何）。
//!
//! 棋盘格背景模拟过场画面，准星按预设参数居中渲染，
//! 描边层 + 四象限分色与 overlay 的 draw_prims 逻辑镜像。

use egui::{pos2, Color32, Rect, Shape, Stroke, Ui};

use crate::config::Preset;
use crate::overlay::parse_hex;
use crate::overlay::shapes::{rotation_safe_radius, Prim, ShapeParams, SLOT_BOTTOM, SLOT_LEFT, SLOT_RIGHT, SLOT_TOP};

/// 在给定区域绘制预览
pub fn paint_preview(ui: &mut Ui, rect: Rect, preset: &Preset) {
    let painter = ui.painter_at(rect);

    // ---- 棋盘格背景 ----
    let cell = 8.0;
    let light = Color32::from_gray(46);
    let dark = Color32::from_gray(38);
    painter.rect_filled(rect, 6.0, dark);
    let mut y = rect.top();
    let mut row = 0i32;
    while y < rect.bottom() {
        let mut x = rect.left();
        let mut col = 0i32;
        while x < rect.right() {
            let c = if (row + col) % 2 == 0 { light } else { dark };
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(x, y),
                    pos2((x + cell).min(rect.right()), (y + cell).min(rect.bottom())),
                ),
                0.0,
                c,
            );
            x += cell;
            col += 1;
        }
        y += cell;
        row += 1;
    }

    // ---- 几何 ----
    let params = ShapeParams::from_preset(preset, 0.0);
    let Some(geom) = crate::overlay::shapes::build(preset.shape, &params) else {
        let lang = crate::i18n::Lang::Zh;
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            crate::ui::strings::t(lang, "custom_image"),
            egui::FontId::proportional(13.0),
            Color32::from_gray(160),
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
        let a255 = (a.clamp(0.0, 1.0) * 255.0) as u8;
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
            let _ = rect;
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