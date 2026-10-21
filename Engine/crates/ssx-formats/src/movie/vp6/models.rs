use super::*;

impl State {
    pub(super) fn default_models(&mut self) {
        let m = &mut self.model;
        m.vector_dct = [0xA2, 0xA4];
        m.vector_sig = [0x80, 0x80];
        m.mb_types_stats = DEF_MB_TYPE_STATS;
        m.vector_fdv = DEF_FDV_MV_MODEL;
        m.vector_pdv = DEF_PDV_MV_MODEL;
        m.coeff_runv = DEF_RUNV_COEFF_MODEL;
        m.coeff_reorder = DEF_COEFF_REORDER;
        m.order_table_init();
    }

    pub(super) fn parse_mb_type_models(&mut self, c: &mut RangeDecoder) {
        let m = &mut self.model;
        for ctx in 0..3 {
            if c.bit(174) {
                let set = c.literal(4) as usize;
                m.mb_types_stats[ctx] = PRE_DEF_MB_TYPE_STATS[set][ctx];
            }
            if c.bit(254) {
                for ty in 0..10 {
                    for i in 0..2 {
                        if c.bit(205) {
                            let negative = c.flag();
                            let mut delta = c.tree(&MB_TYPE_DELTA_TREE, &MB_TYPE_DELTA_PROBS) as i32;
                            if delta == 0 {
                                delta = 4 * c.literal(7) as i32;
                            }
                            let d = if negative { -delta } else { delta };
                            let s = &mut m.mb_types_stats[ctx][ty][i];
                            *s = (i32::from(*s) + d) as u8;
                        }
                    }
                }
            }
        }
        for ctx in 0..3 {
            let stats = m.mb_types_stats[ctx];
            let mut p: [i32; 10] = std::array::from_fn(|t| 100 * i32::from(stats[t][1]));
            for (ty, s) in stats.iter().enumerate() {
                let (s0, s1) = (i32::from(s[0]), i32::from(s[1]));
                let probs = &mut m.mb_type[ctx][ty];
                probs[0] = (255 - (255 * s0) / (1 + s0 + s1)) as u8;
                p[ty] = 0;
                let (p02, p34, p17, p56, p89) = (p[0] + p[2], p[3] + p[4], p[1] + p[7], p[5] + p[6], p[8] + p[9]);
                let (p0234, p5689) = (p02 + p34, p56 + p89);
                let p156789 = p17 + p5689;
                let node = |num: i32, den: i32| (1 + 255 * num / (1 + den)) as u8;
                probs[1] = node(p0234, p0234 + p156789);
                probs[2] = node(p02, p0234);
                probs[3] = node(p17, p156789);
                probs[4] = node(p[0], p02);
                probs[5] = node(p[3], p34);
                probs[6] = node(p[1], p17);
                probs[7] = node(p56, p5689);
                probs[8] = node(p[5], p56);
                probs[9] = node(p[8], p89);
                p[ty] = 100 * s1;
            }
        }
    }

    pub(super) fn parse_vector_models(&mut self, c: &mut RangeDecoder) {
        let m = &mut self.model;
        for comp in 0..2 {
            if c.bit(SIG_DCT_UPDATE[comp][0]) {
                m.vector_dct[comp] = c.prob7();
            }
            if c.bit(SIG_DCT_UPDATE[comp][1]) {
                m.vector_sig[comp] = c.prob7();
            }
        }
        for comp in 0..2 {
            for node in 0..7 {
                if c.bit(PDV_UPDATE[comp][node]) {
                    m.vector_pdv[comp][node] = c.prob7();
                }
            }
        }
        for comp in 0..2 {
            for node in 0..8 {
                if c.bit(FDV_UPDATE[comp][node]) {
                    m.vector_fdv[comp][node] = c.prob7();
                }
            }
        }
    }

    pub(super) fn parse_coeff_models(&mut self, c: &mut RangeDecoder, key: bool, huffman: bool) {
        let m = &mut self.model;
        let mut def_prob = [128u8; 11];
        for pt in 0..2 {
            for node in 0..11 {
                if c.bit(DCCV_UPDATE[pt][node]) {
                    def_prob[node] = c.prob7();
                    m.coeff_dccv[pt][node] = def_prob[node];
                } else if key {
                    m.coeff_dccv[pt][node] = def_prob[node];
                }
            }
        }
        if c.flag() {
            for pos in 1..64 {
                if c.bit(COEFF_REORDER_UPDATE[pos]) {
                    m.coeff_reorder[pos] = c.literal(4) as u8;
                }
            }
            m.order_table_init();
        }
        for cg in 0..2 {
            for node in 0..14 {
                if c.bit(RUNV_UPDATE[cg][node]) {
                    m.coeff_runv[cg][node] = c.prob7();
                }
            }
        }
        for ct in 0..3 {
            for pt in 0..2 {
                for cg in 0..6 {
                    for node in 0..11 {
                        if c.bit(RACT_UPDATE[ct][pt][cg][node]) {
                            def_prob[node] = c.prob7();
                            m.coeff_ract[pt][ct][cg][node] = def_prob[node];
                        } else if key {
                            m.coeff_ract[pt][ct][cg][node] = def_prob[node];
                        }
                    }
                }
            }
        }
        if huffman {
            for pt in 0..2 {
                self.huff_dc[pt] = Tree::build(&m.coeff_dccv[pt], &COEFF_MAP, 12);
                self.huff_run[pt] = Tree::build(&m.coeff_runv[pt], &RUN_MAP, 9);
                for ct in 0..3 {
                    for cg in 0..4 {
                        self.huff_ac[pt][ct][cg] = Tree::build(&m.coeff_ract[pt][ct][cg], &COEFF_MAP, 12);
                    }
                }
            }
            self.nb_null = [[0; 2]; 2];
            return;
        }
        for pt in 0..2 {
            for (ctx, lc) in DCCV_LC.iter().enumerate() {
                for node in 0..5 {
                    let v = ((i32::from(m.coeff_dccv[pt][node]) * lc[node][0] + 128) >> 8) + lc[node][1];
                    m.coeff_dcct[pt][ctx][node] = v.clamp(1, 255) as u8;
                }
            }
        }
    }
}

impl Model {
    fn order_table_init(&mut self) {
        self.index_to_pos[0] = 0;
        let mut idx = 1;
        for band in 0..16 {
            for pos in 1..64 {
                if self.coeff_reorder[pos] == band {
                    self.index_to_pos[idx] = pos as u8;
                    idx += 1;
                }
            }
        }
    }
}
