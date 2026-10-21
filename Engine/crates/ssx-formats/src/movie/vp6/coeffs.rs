use super::*;

impl State {
    pub(super) fn parse_coeff(&mut self, c: &mut RangeDecoder) -> Result<()> {
        if c.past_end() {
            return Err(Error::invalid("VP6 frame: coefficient partition over-read"));
        }
        let m = &self.model;
        for b in 0..6 {
            let pt = usize::from(b > 3);
            let ab = self.above_idx[b];
            let lb = BLOCK_LEFT[b];
            let ctx = usize::from(self.left[lb].not_null_dc) + usize::from(self.above[ab].not_null_dc);
            let mut probs: &[u8] = &m.coeff_dcct[pt][ctx];
            let mut cats: &[u8] = &m.coeff_dccv[pt];
            let mut ct = 1;
            let mut idx = 0usize;
            let block = &mut self.coeffs[b];
            loop {
                let mut run = 1;
                if (idx > 1 && ct == 0) || c.bit(probs[0]) {
                    let mut v = if !c.bit(probs[2]) {
                        ct = 1;
                        1
                    } else {
                        ct = 2;
                        if c.bit(probs[3]) {
                            let cat = c.tree(&TOKEN_CATEGORY_TREE, &cats[6..]);
                            let mut extra = 0;
                            for &p in COEFF_PARSE[cat] {
                                extra = extra << 1 | i32::from(c.bit(p));
                            }
                            COEFF_BIAS[cat + 5] + extra
                        } else if c.bit(probs[4]) {
                            3 + i32::from(c.bit(cats[5]))
                        } else {
                            2
                        }
                    };
                    if c.flag() {
                        v = -v;
                    }
                    if idx > 0 {
                        v *= self.dequant_ac;
                    }
                    block[usize::from(ZIGZAG[usize::from(m.index_to_pos[idx])])] = v as i16;
                } else {
                    ct = 0;
                    if idx > 0 {
                        if !c.bit(probs[1]) {
                            break;
                        }
                        let runv = &m.coeff_runv[usize::from(idx >= 6)];
                        run = c.tree(&RUN_TREE, runv);
                        if run == 0 {
                            run = 9;
                            for i in 0..6 {
                                run += usize::from(c.bit(runv[8 + i])) << i;
                            }
                        }
                    }
                }
                idx += run;
                if idx >= 64 {
                    break;
                }
                let g = &m.coeff_ract[pt][ct][usize::from(COEFF_GROUPS[idx])];
                probs = g;
                cats = g;
            }
            let nz = block[0] != 0;
            self.left[lb].not_null_dc = nz;
            self.above[ab].not_null_dc = nz;
        }
        Ok(())
    }

    pub(super) fn add_dc_predictors(&mut self, ref_frame: u8) {
        for b in 0..6 {
            let ab = &mut self.above[self.above_idx[b]];
            let lb = &mut self.left[BLOCK_LEFT[b]];
            let plane = BLOCK_PLANE[b];
            let mut dc = 0i32;
            let mut n = 0;
            for nb in [&*lb, &*ab] {
                if nb.ref_frame == ref_frame {
                    dc += i32::from(nb.dc);
                    n += 1;
                }
            }
            let pred = match n {
                0 => i32::from(self.prev_dc[plane][usize::from(ref_frame)]),
                1 => dc,
                _ => dc / 2,
            };
            let v = (i32::from(self.coeffs[b][0]) + pred) as i16;
            self.prev_dc[plane][usize::from(ref_frame)] = v;
            *ab = RefDc { dc: v, ref_frame, ..*ab };
            *lb = RefDc { dc: v, ref_frame, ..*lb };
            self.coeffs[b][0] = (i32::from(v) * self.dequant_dc) as i16;
        }
    }
}

impl State {
    pub(super) fn parse_coeff_huffman(&mut self, br: &mut BitReader) -> Result<()> {
        let m = &self.model;
        for b in 0..6 {
            let pt = usize::from(b > 3);
            let mut tree = &self.huff_dc[pt];
            let mut ct = 0;
            let mut idx = 0usize;
            let block = &mut self.coeffs[b];
            loop {
                let mut run = 1;
                if idx < 2 && self.nb_null[idx][pt] > 0 {
                    self.nb_null[idx][pt] -= 1;
                    if idx == 1 {
                        break;
                    }
                } else {
                    if br.left() <= 0 {
                        return Err(Error::invalid("VP6 frame: Huffman partition over-read"));
                    }
                    match tree.read(br) {
                        0 => {
                            ct = 0;
                            if idx > 0 {
                                run += self.huff_run[usize::from(idx >= 6)].read(br);
                                if run >= 9 {
                                    run += br.bits(6) as usize;
                                }
                            } else {
                                self.nb_null[0][pt] = null_count(br);
                            }
                        }
                        11 => {
                            if idx == 1 {
                                self.nb_null[1][pt] = null_count(br);
                            }
                            break;
                        }
                        token => {
                            let mut v = COEFF_BIAS[token];
                            if token > 4 {
                                v += br.bits(if token <= 9 { token as u32 - 4 } else { 11 }) as i32;
                            }
                            ct = 1 + usize::from(v > 1);
                            if br.bit() == 1 {
                                v = -v;
                            }
                            if idx > 0 {
                                v *= self.dequant_ac;
                            }
                            block[usize::from(ZIGZAG[usize::from(m.index_to_pos[idx])])] = v as i16;
                        }
                    }
                }
                idx += run;
                if idx >= 64 {
                    break;
                }
                tree = &self.huff_ac[pt][ct][usize::from(COEFF_GROUPS[idx].min(3))];
            }
        }
        Ok(())
    }
}

fn null_count(br: &mut BitReader) -> u32 {
    match br.bits(2) {
        2 => 2 + br.bits(2),
        3 => {
            let long = br.bit() << 2;
            6 + long + br.bits(2 + long)
        }
        v => v,
    }
}
