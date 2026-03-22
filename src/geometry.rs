#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

impl Size {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn from_corners(a: Point, b: Point) -> Self {
        let x1 = a.x.min(b.x);
        let y1 = a.y.min(b.y);
        let x2 = a.x.max(b.x);
        let y2 = a.y.max(b.y);
        Self {
            x: x1,
            y: y1,
            width: x2 - x1 + 1,
            height: y2 - y1 + 1,
        }
    }

    pub fn clamp_within(self, bounds: Rect) -> Self {
        let width = self.width.clamp(1, bounds.width.max(1));
        let height = self.height.clamp(1, bounds.height.max(1));
        let x = self.x.clamp(bounds.x, bounds.x + bounds.width - width);
        let y = self.y.clamp(bounds.y, bounds.y + bounds.height - height);
        Self {
            x,
            y,
            width,
            height,
        }
    }
}
