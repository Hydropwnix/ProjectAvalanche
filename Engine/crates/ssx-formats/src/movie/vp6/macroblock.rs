use super::*;

impl State {
    fn vector_predictors(&mut self, row: usize, col: usize, ref_frame: u8) -> usize {
        let mut found = [Mv::default(); 2];
        let mut n = 0;
        for (pos, &(dx, dy)) in MV_CANDIDATES.iter().enumerate() {
            let (x, y) = (col as i32 + dx, row as i32 + dy);
            if x < 0 || y < 0 || x >= self.mb_w as i32 {
                continue;
            }
            let m = self.macroblocks[y as usize * self.mb_w + x as usize];
            if REF_FRAME[usize::from(m.ty)] != ref_frame || m.mv == found[0] || m.mv == Mv::default() {
                continue;
            }
            found[n] = m.mv;
            n += 1;
            if n == 2 {
                break;
            }
            self.candidate_pos = pos;
        }
        self.candidates = found;
        [1, 2, 0][n]
    }

    pub(super) fn mb_type(&self, c: &mut RangeDecoder, ctx: usize) -> u8 {
        let probs = &self.model.mb_type[ctx][usize::from(self.prev_type)];
        if c.bit(probs[0]) { self.prev_type } else { c.tree(&MB_TYPE_TREE, &probs[1..]) as u8 }
    }

    fn vector_delta(&self, c: &mut RangeDecoder) -> Mv {
        let m = &self.model;
        let mut v = if self.candidate_pos < 2 { self.candidates[0] } else { Mv::default() };
        for comp in 0..2 {
            let mut delta = if c.bit(m.vector_dct[comp]) {
                let fdv = &m.vector_fdv[comp];
                let mut d = 0;
                for j in [0, 1, 2, 7, 6, 5, 4] {
                    d |= i32::from(c.bit(fdv[j])) << j;
                }
                if d & 0xF0 != 0 { d | i32::from(c.bit(fdv[3])) << 3 } else { d | 8 }
            } else {
                c.tree(&SHORT_MV_TREE, &m.vector_pdv[comp]) as i32
            };
            if delta != 0 && c.bit(m.vector_sig[comp]) {
                delta = -delta;
            }
            if comp == 0 {
                v.x += delta;
            } else {
                v.y += delta;
            }
        }
        v
    }

    pub(super) fn decode_mv(&mut self, c: &mut RangeDecoder, row: usize, col: usize) -> u8 {
        let ctx = self.vector_predictors(row, col, REF_PREVIOUS);
        let ty = self.mb_type(c, ctx);
        self.prev_type = ty;
        let at = row * self.mb_w + col;
        self.macroblocks[at].ty = ty;
        let v = match ty {
            mb::INTER_V1_PF => self.candidates[0],
            mb::INTER_V2_PF => self.candidates[1],
            mb::INTER_V1_GF | mb::INTER_V2_GF => {
                self.vector_predictors(row, col, REF_GOLDEN);
                self.candidates[usize::from(ty == mb::INTER_V2_GF)]
            }
            mb::INTER_DELTA_PF => self.vector_delta(c),
            mb::INTER_DELTA_GF => {
                self.vector_predictors(row, col, REF_GOLDEN);
                self.vector_delta(c)
            }
            mb::INTER_4V => {
                let kinds: [u32; 4] = std::array::from_fn(|_| c.literal(2));
                let mut sum = Mv::default();
                for (b, kind) in kinds.into_iter().enumerate() {
                    self.mv[b] = match kind {
                        0 => Mv::default(),
                        1 => self.vector_delta(c),
                        2 => self.candidates[0],
                        _ => self.candidates[1],
                    };
                    sum.x += self.mv[b].x;
                    sum.y += self.mv[b].y;
                }
                let avg = |s: i32| if s > 0 { (s + 2) >> 2 } else { (s + 1) >> 2 };
                self.mv[4] = Mv { x: avg(sum.x), y: avg(sum.y) };
                self.mv[5] = self.mv[4];
                self.macroblocks[at].mv = self.mv[3];
                return ty;
            }
            _ => Mv::default(),
        };
        self.macroblocks[at].mv = v;
        self.mv = [v; 6];
        ty
    }
}

impl State {
    pub(super) fn render_mb(
        &mut self,
        cur: &mut Picture,
        refs: &[Option<&Picture>; 3],
        row: usize,
        col: usize,
        ty: u8,
    ) -> Result<()> {
        let ref_frame = REF_FRAME[usize::from(ty)];
        self.add_dc_predictors(ref_frame);
        let reference = match ref_frame {
            REF_CURRENT => None,
            r => Some(refs[usize::from(r)].ok_or_else(|| Error::invalid("VP6 frame: missing reference frame"))?),
        };
        for b in 0..6 {
            let plane = BLOCK_PLANE[b];
            let (x, y) = if b < 4 { (col * 16 + (b & 1) * 8, row * 16 + (b >> 1) * 8) } else { (col * 8, row * 8) };
            let (dst, stride) = cur.plane_mut(plane);
            let at = y * stride + x;
            match (reference, ty) {
                (None, _) => dsp::idct(&mut self.coeffs[b], &mut dst[at..], stride, false),
                (Some(r), mb::INTER_NOVEC_PF | mb::INTER_NOVEC_GF) => {
                    let (src, _) = r.plane(plane);
                    for i in 0..8 {
                        dst[at + i * stride..at + i * stride + 8]
                            .copy_from_slice(&src[at + i * stride..at + i * stride + 8]);
                    }
                    dsp::idct(&mut self.coeffs[b], &mut dst[at..], stride, true);
                }
                (Some(r), _) => {
                    let (src, _) = r.plane(plane);
                    let height = src.len() / stride;
                    predict(&mut dst[at..], stride, src, height, x as i32, y as i32, self.mv[b], b < 4);
                    dsp::idct(&mut self.coeffs[b], &mut dst[at..], stride, true);
                }
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn predict(dst: &mut [u8], stride: usize, src: &[u8], height: usize, bx: i32, by: i32, mv: Mv, luma: bool) {
    let div = if luma { 4 } else { 8 };
    let mask = div - 1;
    let (x0, y0) = (bx + mv.x / div - 2, by + mv.y / div - 2);
    let mut win = [0u8; 12 * 12];
    let (w, h) = (stride as i32, height as i32);
    if x0 >= 0 && y0 >= 0 && x0 + 12 <= w && y0 + 12 <= h {
        for r in 0..12 {
            let s = (y0 + r) as usize * stride + x0 as usize;
            win[r as usize * 12..r as usize * 12 + 12].copy_from_slice(&src[s..s + 12]);
        }
    } else {
        for r in 0..12 {
            let sy = (y0 + r).clamp(0, h - 1) as usize * stride;
            for c in 0..12 {
                win[(r * 12 + c) as usize] = src[sy + (x0 + c).clamp(0, w - 1) as usize];
            }
        }
    }
    let step = |v: i32| if v & mask == 0 { 0 } else { v.signum() };
    let (ox, oy) = (step(mv.x), step(mv.y));
    if ox == 0 && oy == 0 {
        for r in 0..8 {
            dst[r * stride..r * stride + 8].copy_from_slice(&win[(r + 2) * 12 + 2..(r + 2) * 12 + 10]);
        }
        return;
    }
    let scale = if luma { 2 } else { 1 };
    let (fx, fy) = ((mv.x & mask) * scale, (mv.y & mask) * scale);
    let (mut px, mut py) = (2i32, 2i32);
    if (fy != 0 && oy < 0) || (fy == 0 && ox < 0) {
        px += ox;
        py += oy;
    }
    if fx == 0 || fy == 0 {
        dsp::bilinear(dst, stride, &win[(py * 12 + px) as usize..], 12, fx, fy);
    } else {
        let px = px - i32::from((mv.x ^ mv.y) < 0);
        dsp::bilinear_2pass(dst, stride, &win[(py * 12 + px) as usize..], 12, fx, fy);
    }
}
