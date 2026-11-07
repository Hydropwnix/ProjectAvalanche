use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};

pub fn read_pose_blocks(path: &std::path::Path) -> Result<(String, Vec<PoseBlock>), String> {
    let name = path.display();
    let d = std::fs::read(path).map_err(|e| format!("{name}: {e}"))?;
    if d.len() < 10 || &d[..4] != b"PBLK" || d[4..8] != 1u32.to_le_bytes() {
        return Err(format!("{name}: not a version 1 pose block file"));
    }
    let n = usize::from(u16::from_le_bytes([d[8], d[9]]));
    let track = d.get(10..10 + n).ok_or_else(|| format!("{name}: the track name is cut"))?;
    let track = String::from_utf8(track.to_vec()).map_err(|_| format!("{name}: the track name is not UTF-8"))?;
    let (records, rest) = d[10 + n..].as_chunks::<{ 8 + POSE_BLOCK_SIZE }>();
    if !rest.is_empty() {
        return Err(format!("{name}: the records are not whole"));
    }
    let mut blocks = Vec::with_capacity(records.len());
    for (k, r) in records.iter().enumerate() {
        if u32::from_le_bytes([r[0], r[1], r[2], r[3]]) as usize != k {
            return Err(format!("{name}: record {k} is not execution {k}"));
        }
        blocks.push(PoseBlock::parse(&r[8..]).map_err(|e| format!("{name}: record {k}: {e}"))?);
    }
    Ok((track, blocks))
}
