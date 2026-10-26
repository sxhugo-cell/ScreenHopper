//! Small rectangle helpers for multi-monitor layout maths.
use crate::platform::Rect;

pub fn contains(r: &Rect, x: f64, y: f64) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

pub fn rect_at(rs: &[Rect], x: f64, y: f64) -> Option<Rect> {
    rs.iter().find(|r| contains(r, x, y)).copied()
}

pub fn shifted(rs: &[Rect], dx: f64, dy: f64) -> Vec<Rect> {
    rs.iter().map(|r| Rect { x: r.x + dx, y: r.y + dy, w: r.w, h: r.h }).collect()
}

/// Clamp a point into a rectangle, keeping `inset` pixels from every side.
pub fn clamp_in(r: &Rect, x: f64, y: f64, inset: f64) -> (f64, f64) {
    let i = inset.min(r.w / 2.0).min(r.h / 2.0);
    (x.clamp(r.x + i, r.x + r.w - 1.0 - i), y.clamp(r.y + i, r.y + r.h - 1.0 - i))
}

/// Closest point to (x, y) that lies inside any rectangle.
pub fn nearest(rs: &[Rect], x: f64, y: f64) -> (f64, f64) {
    let mut best = (x, y);
    let mut bd = f64::MAX;
    for r in rs {
        let (cx, cy) = clamp_in(r, x, y, 0.0);
        let d = (cx - x).powi(2) + (cy - y).powi(2);
        if d < bd {
            bd = d;
            best = (cx, cy);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    #[test]
    fn point_maths() {
        let ms = [r(0.0, 0.0, 100.0, 100.0), r(100.0, 0.0, 100.0, 50.0)];
        assert!(rect_at(&ms, 150.0, 10.0).is_some());
        assert!(rect_at(&ms, 150.0, 60.0).is_none());
        assert_eq!(nearest(&ms, 150.0, 80.0), (150.0, 49.0));
        assert_eq!(clamp_in(&ms[0], -5.0, 50.0, 6.0), (6.0, 50.0));
    }
}
