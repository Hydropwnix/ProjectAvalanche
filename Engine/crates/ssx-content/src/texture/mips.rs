use super::{Level, chain_len};

fn footprints(from: u32, to: u32) -> Vec<Vec<(usize, u64)>> {
    let (from, to) = (u64::from(from), u64::from(to));
    (0..to)
        .map(|d| {
            let (start, end) = (d * from, (d + 1) * from);
            (start / to..end.div_ceil(to))
                .filter_map(|s| {
                    let share = end.min((s + 1) * to) - start.max(s * to);
                    (share > 0).then_some((s as usize, share))
                })
                .collect()
        })
        .collect()
}

// TODO(guess): the mean is rounded half up, and an odd size's footprint is area-weighted; the disc's mips are box
// means of stored values (example mip_rule: closest in 1 976 of 1 978 DXT1, 321 of 328 DXN textures), but their BC
// quantisation (mean |error| 2.4–9) hides whether EA's tool rounded or truncated, and no odd-sized disc texture has
// mips; settle: EA's texture tool (the disc's one uncompressed texture with mips has hand-made levels).
pub fn reduce(image: &[u8], width: u32, height: u32, w: u32, h: u32) -> Vec<u8> {
    let (xs, ys) = (footprints(width, w), footprints(height, h));
    let stride = width as usize * 4;
    let area = u64::from(width) * u64::from(height);
    let mut out = Vec::with_capacity(w as usize * h as usize * 4);
    for ytaps in &ys {
        for xtaps in &xs {
            let mut sum = [0u64; 4];
            for &(sy, wy) in ytaps {
                for &(sx, wx) in xtaps {
                    let at = sy * stride + sx * 4;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += u64::from(image[at + c]) * wx * wy;
                    }
                }
            }
            out.extend(sum.map(|s| ((s + area / 2) / area) as u8));
        }
    }
    out
}

pub fn build_chain(level0: Level) -> Vec<Level> {
    let n = chain_len(level0.width, level0.height) as usize;
    let mut levels = Vec::with_capacity(n);
    levels.push(level0);
    while levels.len() < n {
        let Some(prev) = levels.last() else { break };
        let (w, h) = ((prev.width / 2).max(1), (prev.height / 2).max(1));
        let images = prev.images.iter().map(|i| reduce(i, prev.width, prev.height, w, h)).collect();
        levels.push(Level { width: w, height: h, images });
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halving_is_the_2x2_mean_rounded_half_up() {
        let img = [0, 0, 10, 255, 1, 0, 20, 255, 1, 0, 30, 255, 1, 2, 40, 0];
        assert_eq!(reduce(&img, 2, 2, 1, 1), [1, 1, 25, 191]);
    }

    #[test]
    fn odd_sizes_weigh_the_covered_share() {
        assert_eq!(reduce(&[0, 0, 0, 0, 30, 30, 30, 30, 60, 60, 60, 60], 3, 1, 1, 1), [30; 4]);
        let row: Vec<u8> = [0u8, 10, 20, 30, 40].iter().flat_map(|&v| [v; 4]).collect();
        assert_eq!(reduce(&row, 5, 1, 2, 1), [8, 8, 8, 8, 32, 32, 32, 32]);
    }

    #[test]
    fn every_size_stays_inside_the_image() {
        for size in (1..=300).chain([991, 981, 1023, 1025, 4095, 4096]) {
            for (from, to) in [(size, (size / 2).max(1)), (size, 1)] {
                let f = footprints(from, to);
                assert_eq!(f.len(), to as usize);
                for taps in &f {
                    assert!(taps.iter().all(|&(s, _)| s < from as usize), "{from} -> {to}");
                    assert_eq!(taps.iter().map(|t| t.1).sum::<u64>(), u64::from(from));
                }
            }
        }
        let (w, h) = (991, 981);
        let flat = vec![77u8; w * h * 4];
        let chain = build_chain(Level { width: w as u32, height: h as u32, images: vec![flat] });
        assert_eq!(chain.len(), 10);
        assert!(chain.iter().all(|l| l.images[0].iter().all(|&v| v == 77)));
    }

    #[test]
    fn chains_end_at_one_by_one() {
        let l0 = Level { width: 8, height: 2, images: vec![vec![200; 8 * 2 * 4]; 6] };
        let chain = build_chain(l0);
        let shapes: Vec<_> = chain.iter().map(|l| (l.width, l.height, l.images.len())).collect();
        assert_eq!(shapes, [(8, 2, 6), (4, 1, 6), (2, 1, 6), (1, 1, 6)]);
        assert!(chain.iter().all(|l| l.images.iter().all(|i| i.iter().all(|&v| v == 200))));
    }
}
