use std::io::Read;

mod child;
mod parent;

pub use child::{ChildSession, enter_child, serve, serve_measured};
pub use parent::{HANG, Isolated, Out, find_program, program};

pub use crate::isolate_os::{MAX_THREADS, MEMORY_LIMIT};

pub const PROGRAM: &str = if cfg!(windows) { "avalanche-movie.exe" } else { "avalanche-movie" };

const MAGIC: &[u8; 8] = b"SSXAV1\x00\x01";
const MAX_UNIT: u32 = 64 << 20;

fn read_exact(r: &mut impl Read, n: usize) -> Result<Vec<u8>, String> {
    let mut b = vec![0u8; n];
    r.read_exact(&mut b).map_err(|e| format!("the pipe: {e}"))?;
    Ok(b)
}

fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
