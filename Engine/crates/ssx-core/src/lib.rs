pub mod master;
pub mod output;
pub mod print;
pub mod text;

pub use master::OFFICIAL_MASTER;

pub const WORLD_UP: [f32; 3] = [0.0, 1.0, 0.0];

// PORT(choice): 2^17 m, the bound of the port's maps and session samples, from the 8 mm collision grid in f32; every
// coordinate of the disc's worlds lies within (the farthest 73 044 m); the console had no maps and no bound.
pub const WORLD_EXTENT_M: f32 = 131_072.0;
