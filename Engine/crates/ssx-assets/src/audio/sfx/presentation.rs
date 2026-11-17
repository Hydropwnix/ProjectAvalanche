pub const SKYDROP: i32 = 1;
pub const GROUND: i32 = 2;

pub fn tutorial(t: i32) -> bool {
    matches!(t, SKYDROP | GROUND)
}
