use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn area(&self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);

        if right <= left || bottom <= top {
            return None;
        }
        Some(Rect::new(left, top, right - left, bottom - top))
    }

    pub fn iou(&self, other: &Rect) -> f32 {
        let inter = match self.intersection(other) {
            Some(r) => r.area(),
            None => return 0.0,
        };
        let union = self.area() + other.area() - inter;
        if union <= 0.0 {
            0.0
        } else {
            inter / union
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_rects_have_full_overlap() {
        let r = Rect::new(10.0, 10.0, 50.0, 20.0);
        assert!((r.iou(&r) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn disjoint_rects_do_not_overlap() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 20.0, 10.0, 10.0);
        assert!(a.intersection(&b).is_none());
        assert_eq!(a.iou(&b), 0.0);
    }

    #[test]
    fn partial_overlap_is_computed_correctly() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 0.0, 10.0, 10.0);
        // intersection 50, union 150
        assert!((a.iou(&b) - 1.0 / 3.0).abs() < 1e-6);
    }
}
