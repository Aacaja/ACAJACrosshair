//! 准星形状几何（纯数学，不依赖 Windows，可单元测试）。
//!
//! 坐标系：所有图元以 (0,0) 为中心；旋转、屏幕偏移由渲染层处理。
//! 颜色槽位：`SLOT_MAIN` 单色；四象限形状使用 TOP/BOTTOM/LEFT/RIGHT 分色
//! （对齐 Crosshair X 的多色准星）。

use crate::config::Shape;

/// 颜色槽位
pub const SLOT_MAIN: u8 = 0;
pub const SLOT_TOP: u8 = 1;
pub const SLOT_BOTTOM: u8 = 2;
pub const SLOT_LEFT: u8 = 3;
pub const SLOT_RIGHT: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// 渲染图元
#[derive(Clone, Debug, PartialEq)]
pub enum Prim {
    Line { x1: f32, y1: f32, x2: f32, y2: f32, slot: u8 },
    RectFill { cx: f32, cy: f32, w: f32, h: f32, slot: u8 },
    RectStroke { cx: f32, cy: f32, w: f32, h: f32, slot: u8 },
    Dot { cx: f32, cy: f32, r: f32, slot: u8 },
    Ring { cx: f32, cy: f32, r: f32, slot: u8 },
    PolyFill { pts: Vec<Point>, slot: u8 },
}

/// 形状几何 + 未旋转包围盒
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeGeom {
    pub prims: Vec<Prim>,
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

/// 形状参数（从预设映射而来）
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeParams {
    pub size: f32,
    pub thickness: f32,
    pub gap: f32,
    pub dot: f32,
    /// 动态扩散量（开火时增大缺口/臂长）
    pub expand: f32,
    /// 描边厚度（用于包围盒留白）
    pub outline: f32,
}

impl Default for ShapeParams {
    fn default() -> Self {
        ShapeParams { size: 20.0, thickness: 2.0, gap: 0.0, dot: 3.0, expand: 0.0, outline: 0.0 }
    }
}

impl ShapeParams {
    /// 臂长（十字/空心十字）：半臂 = size，扩散时加长
    fn arm(self) -> f32 {
        self.size
    }
    /// 缺口：gap + 扩散量（开火时缺口变大）
    fn eff_gap(self) -> f32 {
        self.gap + self.expand
    }
    /// 外接半径（考虑粗细与描边）
    fn radius(self) -> f32 {
        let r = self.size.max(self.gap + self.size) + self.thickness / 2.0 + self.outline + 1.0;
        r + self.expand
    }
}

/// 生成形状几何；`CustomImage` 返回 `None`（由渲染层处理位图）
pub fn build(shape: Shape, p: &ShapeParams) -> Option<ShapeGeom> {
    let mut prims = Vec::new();
    match shape {
        Shape::Cross => {
            // 四象限分色：每轴线拆成两段
            let s = p.arm() + p.expand;
            prims.push(Prim::Line { x1: -s, y1: 0.0, x2: 0.0, y2: 0.0, slot: SLOT_LEFT });
            prims.push(Prim::Line { x1: 0.0, y1: 0.0, x2: s, y2: 0.0, slot: SLOT_RIGHT });
            prims.push(Prim::Line { x1: 0.0, y1: -s, x2: 0.0, y2: 0.0, slot: SLOT_TOP });
            prims.push(Prim::Line { x1: 0.0, y1: 0.0, x2: 0.0, y2: s, slot: SLOT_BOTTOM });
        }
        Shape::Dot => {
            prims.push(Prim::Dot { cx: 0.0, cy: 0.0, r: (p.size + p.expand) / 2.0, slot: SLOT_MAIN });
        }
        Shape::Square => {
            let s = p.size + p.expand;
            prims.push(Prim::RectFill { cx: 0.0, cy: 0.0, w: s, h: s, slot: SLOT_MAIN });
        }
        Shape::Circle => {
            prims.push(Prim::Ring { cx: 0.0, cy: 0.0, r: (p.size + p.expand) / 2.0, slot: SLOT_MAIN });
        }
        Shape::HollowCross => {
            let s = p.arm() + p.expand;
            let g = p.eff_gap();
            arms_quad(&mut prims, g, s);
        }
        Shape::HollowSquare => {
            let s = p.size + p.expand;
            prims.push(Prim::RectStroke { cx: 0.0, cy: 0.0, w: s, h: s, slot: SLOT_MAIN });
        }
        Shape::HollowCrossDot => {
            let s = p.arm() + p.expand;
            let g = p.eff_gap();
            arms_quad(&mut prims, g, s);
            prims.push(Prim::Dot { cx: 0.0, cy: 0.0, r: p.dot / 2.0, slot: SLOT_MAIN });
        }
        Shape::GapHair => {
            let g = p.eff_gap().max(2.0);
            let s = p.arm() + p.expand;
            arms_quad(&mut prims, g, s);
            prims.push(Prim::Dot { cx: 0.0, cy: 0.0, r: p.dot / 2.0, slot: SLOT_MAIN });
        }
        Shape::Chevron => {
            // 上指向角（^）：左臂/右臂分色
            let s = p.size + p.expand;
            let h = s * 0.5;
            prims.push(Prim::Line { x1: -s, y1: h, x2: 0.0, y2: -h, slot: SLOT_LEFT });
            prims.push(Prim::Line { x1: 0.0, y1: -h, x2: s, y2: h, slot: SLOT_RIGHT });
            // 顶点拼接点（消除粗线条 butt cap 内角缺口）
            joint_dot(&mut prims, 0.0, -h, p.thickness, SLOT_LEFT);
        }
        Shape::VShape => {
            let s = p.size + p.expand;
            let h = s * 0.5;
            prims.push(Prim::Line { x1: 0.0, y1: -h, x2: -s, y2: h, slot: SLOT_LEFT });
            prims.push(Prim::Line { x1: 0.0, y1: -h, x2: s, y2: h, slot: SLOT_RIGHT });
            joint_dot(&mut prims, 0.0, -h, p.thickness, SLOT_LEFT);
        }
        Shape::TShape => {
            let s = p.size + p.expand;
            let h = s * 0.5;
            prims.push(Prim::Line { x1: -s, y1: -h, x2: s, y2: -h, slot: SLOT_MAIN });
            prims.push(Prim::Line { x1: 0.0, y1: -h, x2: 0.0, y2: h, slot: SLOT_MAIN });
        }
        Shape::Brackets => {
            // 四角括号
            let s = p.size + p.expand;
            let g = s * 0.35;
            // 左上
            prims.push(Prim::Line { x1: -s, y1: -g, x2: -s, y2: -s, slot: SLOT_MAIN });
            prims.push(Prim::Line { x1: -g, y1: -s, x2: -s, y2: -s, slot: SLOT_MAIN });
            // 右上
            prims.push(Prim::Line { x1: g, y1: -s, x2: s, y2: -s, slot: SLOT_MAIN });
            prims.push(Prim::Line { x1: s, y1: -g, x2: s, y2: -s, slot: SLOT_MAIN });
            // 左下
            prims.push(Prim::Line { x1: -s, y1: g, x2: -s, y2: s, slot: SLOT_MAIN });
            prims.push(Prim::Line { x1: -g, y1: s, x2: -s, y2: s, slot: SLOT_MAIN });
            // 右下
            prims.push(Prim::Line { x1: g, y1: s, x2: s, y2: s, slot: SLOT_MAIN });
            prims.push(Prim::Line { x1: s, y1: g, x2: s, y2: s, slot: SLOT_MAIN });
        }
        Shape::Triangle => {
            let s = p.size + p.expand;
            let pts = vec![
                Point { x: 0.0, y: -s * 0.6 },
                Point { x: -s * 0.5, y: s * 0.5 },
                Point { x: s * 0.5, y: s * 0.5 },
            ];
            prims.push(Prim::PolyFill { pts, slot: SLOT_MAIN });
        }
        Shape::CustomImage => return None,
    }

    let mut g = ShapeGeom {
        prims,
        min_x: f32::MAX,
        min_y: f32::MAX,
        max_x: f32::MIN,
        max_y: f32::MIN,
    };
    compute_bounds(&mut g, p);
    // 中心留一点安全边距
    g.min_x -= 1.0;
    g.min_y -= 1.0;
    g.max_x += 1.0;
    g.max_y += 1.0;
    Some(g)
}

/// 空心十字四臂（带四象限分色）
fn arms_quad(prims: &mut Vec<Prim>, gap: f32, arm_len: f32) {
    prims.push(Prim::Line { x1: 0.0, y1: -gap - arm_len, x2: 0.0, y2: -gap, slot: SLOT_TOP });
    prims.push(Prim::Line { x1: 0.0, y1: gap, x2: 0.0, y2: gap + arm_len, slot: SLOT_BOTTOM });
    prims.push(Prim::Line { x1: -gap - arm_len, y1: 0.0, x2: -gap, y2: 0.0, slot: SLOT_LEFT });
    prims.push(Prim::Line { x1: gap, y1: 0.0, x2: gap + arm_len, y2: 0.0, slot: SLOT_RIGHT });
}

/// 角点拼接圆点（盖住粗线 butt cap 的内角缺口）
fn joint_dot(prims: &mut Vec<Prim>, x: f32, y: f32, thickness: f32, slot: u8) {
    prims.push(Prim::Dot { cx: x, cy: y, r: thickness * 0.55, slot });
}

fn compute_bounds(g: &mut ShapeGeom, p: &ShapeParams) {
    let m = p.thickness / 2.0 + p.outline;
    for prim in &g.prims {
        match prim {
            Prim::Line { x1, y1, x2, y2, .. } => {
                g.min_x = g.min_x.min((*x1).min(*x2) - m);
                g.min_y = g.min_y.min((*y1).min(*y2) - m);
                g.max_x = g.max_x.max((*x1).max(*x2) + m);
                g.max_y = g.max_y.max((*y1).max(*y2) + m);
            }
            Prim::RectFill { cx, cy, w, h, .. } | Prim::RectStroke { cx, cy, w, h, .. } => {
                let hw = *w / 2.0 + m;
                let hh = *h / 2.0 + m;
                g.min_x = g.min_x.min(*cx - hw);
                g.min_y = g.min_y.min(*cy - hh);
                g.max_x = g.max_x.max(*cx + hw);
                g.max_y = g.max_y.max(*cy + hh);
            }
            Prim::Dot { cx, cy, r, .. } | Prim::Ring { cx, cy, r, .. } => {
                g.min_x = g.min_x.min(*cx - r - m);
                g.min_y = g.min_y.min(*cy - r - m);
                g.max_x = g.max_x.max(*cx + r + m);
                g.max_y = g.max_y.max(*cy + r + m);
            }
            Prim::PolyFill { pts, .. } => {
                for pt in pts {
                    g.min_x = g.min_x.min(pt.x);
                    g.min_y = g.min_y.min(pt.y);
                    g.max_x = g.max_x.max(pt.x);
                    g.max_y = g.max_y.max(pt.y);
                }
            }
        }
    }
    // 处理空几何的退化值
    if g.prims.is_empty() {
        g.min_x = 0.0;
        g.min_y = 0.0;
        g.max_x = 0.0;
        g.max_y = 0.0;
    }
}

/// 几何外接圆半径（旋转安全半径 = 对角半径 × √2）
pub fn rotation_safe_radius(g: &ShapeGeom) -> f32 {
    let rx = g.max_x.abs().max(g.min_x.abs());
    let ry = g.max_y.abs().max(g.min_y.abs());
    (rx * rx + ry * ry).sqrt() * std::f32::consts::SQRT_2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> ShapeParams {
        ShapeParams { size: 20.0, thickness: 2.0, gap: 0.0, dot: 3.0, expand: 0.0, outline: 0.0 }
    }

    #[test]
    fn cross_has_four_quadrant_arms() {
        let g = build(Shape::Cross, &params()).unwrap();
        assert_eq!(g.prims.len(), 4);
        let slots: Vec<u8> = g.prims.iter().map(|p| match p {
            Prim::Line { slot, .. } => *slot,
            _ => unreachable!(),
        }).collect();
        assert!(slots.contains(&SLOT_TOP));
        assert!(slots.contains(&SLOT_BOTTOM));
        assert!(slots.contains(&SLOT_LEFT));
        assert!(slots.contains(&SLOT_RIGHT));
        // 十字半臂 20，左右各到 ±20（+粗细边距）
        assert!((g.max_x - 20.0).abs() < 3.0);
        assert!((g.max_y - 20.0).abs() < 3.0);
        // 分色：每段都不跨越原点一半（左臂终点=0）
        let left = g.prims.iter().find_map(|p| match p {
            Prim::Line { x1, x2, slot: SLOT_LEFT, .. } => Some((*x1, *x2)),
            _ => None,
        }).unwrap();
        assert!(left.0 < 0.0 && left.1 == 0.0);
    }

    #[test]
    fn hollow_cross_uses_gap() {
        let mut p = params();
        p.gap = 8.0;
        let g = build(Shape::HollowCross, &p).unwrap();
        assert_eq!(g.prims.len(), 4);
        // 顶部臂：从 -gap 到 -gap-arm
        let top = g.prims.iter().find_map(|prim| match prim {
            Prim::Line { y1, y2, slot: SLOT_TOP, .. } => Some((*y1, *y2)),
            _ => None,
        }).unwrap();
        let (a, b) = if top.0 < top.1 { (top.0, top.1) } else { (top.1, top.0) };
        assert!((a - (-28.0)).abs() < 0.01, "a={a}");
        assert!((b - (-8.0)).abs() < 0.01, "b={b}");
    }

    #[test]
    fn expand_widens_gap() {
        let mut p = params();
        p.gap = 4.0;
        p.expand = 6.0;
        let g = build(Shape::HollowCross, &p).unwrap();
        let top = g.prims.iter().find_map(|prim| match prim {
            Prim::Line { y2, slot: SLOT_TOP, .. } => Some(*y2),
            _ => None,
        }).unwrap();
        assert!((top - (-10.0)).abs() < 0.01); // gap 4 + expand 6 = 10
    }

    #[test]
    fn dot_square_circle_shapes() {
        let p = params();
        let dot = build(Shape::Dot, &p).unwrap();
        assert!(matches!(&dot.prims[0], Prim::Dot { r, .. } if (*r - 10.0).abs() < 0.01));

        let sq = build(Shape::Square, &p).unwrap();
        assert!(matches!(&sq.prims[0], Prim::RectFill { w, h, .. } if *w == 20.0 && *h == 20.0));

        let ring = build(Shape::Circle, &p).unwrap();
        assert!(matches!(&ring.prims[0], Prim::Ring { .. }));
    }

    #[test]
    fn hollow_cross_dot_has_center() {
        let p = params();
        let g = build(Shape::HollowCrossDot, &p).unwrap();
        assert_eq!(g.prims.len(), 5);
        assert!(matches!(&g.prims[4], Prim::Dot { r, .. } if (*r - 1.5).abs() < 0.01));
    }

    #[test]
    fn brackets_have_eight_lines() {
        let g = build(Shape::Brackets, &params()).unwrap();
        assert_eq!(g.prims.len(), 8);
        assert!((g.max_x - 20.0).abs() < 3.0);
    }

    #[test]
    fn chevron_has_joint_dot() {
        let g = build(Shape::Chevron, &params()).unwrap();
        // 2 臂 + 1 拼接点
        assert_eq!(g.prims.len(), 3);
        assert!(matches!(&g.prims[2], Prim::Dot { .. }));
    }

    #[test]
    fn triangle_is_polyfill() {
        let g = build(Shape::Triangle, &params()).unwrap();
        assert!(matches!(&g.prims[0], Prim::PolyFill { pts, .. } if pts.len() == 3));
    }

    #[test]
    fn custom_image_returns_none() {
        assert!(build(Shape::CustomImage, &params()).is_none());
    }

    #[test]
    fn rotation_radius_covers_bounds() {
        let g = build(Shape::Cross, &params()).unwrap();
        // 半臂 20 + 粗细边距 1 + 安全边距 1 = 22 → 旋转安全半径 ≈ 22·√2·1.414…
        let r = rotation_safe_radius(&g);
        assert!(r > 35.0 && r < 60.0, "r={r}");
        // 旋转后必然覆盖未旋转包围盒
        assert!(r >= (g.max_x.abs() * std::f32::consts::SQRT_2));
    }

    #[test]
    fn bounds_include_thickness() {
        let mut p = params();
        p.thickness = 10.0;
        let g = build(Shape::Cross, &p).unwrap();
        // 半臂 20 + 粗细/2 边距 5 + 安全边距 1 = 26
        assert!((g.max_y - 26.0).abs() < 0.01, "max_y={}", g.max_y);
    }
}