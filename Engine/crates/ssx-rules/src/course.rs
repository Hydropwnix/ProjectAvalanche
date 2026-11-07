pub const SPLITS: usize = 9;
pub const SPLIT_TOP_OFFSET: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Course {
    pub start_y: f32,
    pub end_y: f32,
}

impl Course {
    pub fn splits(&self) -> [f32; SPLITS] {
        let top = self.start_y - SPLIT_TOP_OFFSET;
        let step = (top - self.end_y) * 0.111_111_1;
        std::array::from_fn(|k| top - step * k as f32)
    }

    pub fn split_index(&self, y: f32) -> i32 {
        self.splits().iter().rposition(|h| y < *h).map_or(-1, |k| k as i32)
    }

    pub fn progress(&self, y: f32) -> f32 {
        let d = self.start_y - y;
        let p = if d > 0.0 { d / (self.start_y - self.end_y) } else { 0.0 };
        p.clamp(0.0, 1.0)
    }
}
