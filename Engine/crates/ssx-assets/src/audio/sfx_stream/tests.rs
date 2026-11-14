use super::*;

fn v0(n: usize, samples: u32, fill: u8, last: bool) -> Vec<u8> {
    let size = (8 + n) as u32 | if last { 0x8000_0000 } else { 0 };
    let mut b = size.to_be_bytes().to_vec();
    b.extend(samples.to_be_bytes());
    b.extend(std::iter::repeat_n(fill, n));
    b
}

fn sps(id: u8, body: &[u8]) -> Vec<u8> {
    let mut b = (u32::from(id) << 24 | (4 + body.len()) as u32).to_be_bytes().to_vec();
    b.extend(body);
    b
}

fn drain(slot: &mut Slot) -> Vec<(BlockType, u32, usize, u8, bool)> {
    let mut out = Vec::new();
    loop {
        match slot.next_block() {
            Next::Block(b) => {
                out.push((b.ty, b.samples, b.payload.len(), b.payload.first().copied().unwrap_or(0), b.last));
                if b.last {
                    return out;
                }
            }
            other => panic!("{other:?} after {} blocks", out.len()),
        }
    }
}

#[test]
fn chunk_sizes_follow_the_strm_constructor() {
    assert_eq!(Ring::SFX_LARGE.chunk(), 64 << 10);
    assert_eq!(Ring::MUSIC.chunk(), 26_624);
    assert_eq!(Ring::SPEECH.chunk(), 12_288);
    assert_eq!((Ring::SFX_LARGE.chunks(), Ring::MUSIC.chunks(), Ring::SPEECH.chunks()), (6, 5, 6));
}

#[test]
fn version_0_blocks_across_chunks() {
    let mut file = vec![0xEE; 16];
    for k in 0..9u8 {
        file.extend(v0(5000 + usize::from(k), u32::from(k) * 10, k, k == 8));
    }
    file.extend(v0(100, 1, 0x77, true));
    let f = StreamFile::memory("v0", file);
    let mut slot = Slot::new(Ring::SPEECH).unwrap();
    slot.queue(f, 16, BlockKind::V0);
    let got = drain(&mut slot);
    assert_eq!(got.len(), 9);
    for (k, b) in got.iter().enumerate() {
        assert_eq!(*b, (BlockType::Data, k as u32 * 10, 5000 + k, k as u8, k == 8));
    }
    assert!(matches!(slot.next_block(), Next::Idle));
}

#[test]
fn sps_blocks() {
    let mut file = sps(b'H', &[1, 2, 3, 4, 5, 6, 7, 8]);
    let mut d = 77u32.to_be_bytes().to_vec();
    d.extend([9; 30]);
    file.extend(sps(b'D', &d));
    file.extend(sps(b'E', &[]));
    let mut slot = Slot::new(Ring::MUSIC).unwrap();
    slot.queue(StreamFile::memory("sps", file), 0, BlockKind::Sps);
    let got = drain(&mut slot);
    assert_eq!(
        got,
        [(BlockType::Header, 0, 8, 1, false), (BlockType::Data, 77, 30, 9, false), (BlockType::End, 0, 0, 0, true)]
    );
}

#[test]
fn requests_skip_and_flush() {
    let mut file = Vec::new();
    for k in 0..3u8 {
        for j in 0..20u8 {
            file.extend(v0(3000, 1, k * 32 + j, j == 19));
        }
    }
    let f = StreamFile::memory("three", file);
    let one = 20 * 3008;
    let mut slot = Slot::new(Ring::SPEECH).unwrap();
    let a = slot.queue(f.clone(), 0, BlockKind::V0);
    let b = slot.queue(f.clone(), one, BlockKind::V0);
    match slot.next_block() {
        Next::Block(blk) => assert_eq!((blk.tag, blk.payload[0]), (a, 0)),
        other => panic!("{other:?}"),
    }
    slot.skip_front();
    assert_eq!(slot.front(), Some(b));
    let got = drain(&mut slot);
    assert_eq!((got.len(), got[0].3, got[19].3), (20, 32, 51));
    slot.queue(f.clone(), 2 * one, BlockKind::V0);
    slot.flush();
    assert!(matches!(slot.next_block(), Next::Idle));
    slot.queue(f, 2 * one, BlockKind::V0);
    assert_eq!(drain(&mut slot)[0].3, 64);
}

#[test]
fn bad_streams_fail() {
    let mut short = v0(10, 1, 0, false);
    short.extend(4u32.to_be_bytes());
    short.extend([0; 8]);
    let mut huge = (0x0010_0000u32).to_be_bytes().to_vec();
    huge.extend([0; 12]);
    let cut = v0(4000, 1, 0, false);
    for (name, bytes) in [("short", short), ("huge", huge), ("cut", cut)] {
        let mut slot = Slot::new(Ring::SPEECH).unwrap();
        slot.queue(StreamFile::memory(name, bytes), 0, BlockKind::V0);
        let mut seen = 0;
        loop {
            match slot.next_block() {
                Next::Block(_) => seen += 1,
                Next::Failed { .. } => break,
                other => panic!("{name}: {other:?}"),
            }
            assert!(seen < 4, "{name}");
        }
        assert!(matches!(slot.next_block(), Next::Idle), "{name}");
    }
}

#[test]
fn live_thread_never_waits() {
    let mut file = Vec::new();
    for k in 0..200u8 {
        file.extend(v0(9000, 1, k, k == 199));
    }
    let f = StreamFile::memory("live", file);
    std::thread::spawn(move || {
        mark_live_thread();
        let mut slot = Slot::new(Ring::SPEECH).unwrap();
        slot.queue(f, 0, BlockKind::V0);
        let (mut blocks, mut polls) = (0u32, 0u32);
        let start = std::time::Instant::now();
        loop {
            polls += 1;
            match slot.next_block() {
                Next::Block(b) => {
                    assert_eq!(b.payload[0], blocks as u8);
                    blocks += 1;
                    if b.last {
                        break;
                    }
                }
                Next::Pending => std::thread::yield_now(),
                other => panic!("{other:?}"),
            }
            assert!(start.elapsed().as_secs() < 20);
        }
        assert_eq!(blocks, 200);
        assert!(polls >= blocks);
    })
    .join()
    .unwrap();
}
