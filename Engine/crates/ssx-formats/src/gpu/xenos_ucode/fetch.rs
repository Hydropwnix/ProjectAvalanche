use super::*;

fn texture_fetch_name(opcode: u32, dimension: u32) -> Option<String> {
    const DIMS: [&str; 4] = ["1D", "2D", "3D", "Cube"];
    let d = DIMS[dimension as usize & 3];
    Some(match opcode {
        1 => format!("tfetch{d}"),
        16 => format!("getBCF{d}"),
        17 => format!("getCompTexLOD{d}"),
        18 => "getGradients".to_string(),
        19 => format!("getWeights{d}"),
        24 => "setTexLOD".to_string(),
        25 => "setGradientH".to_string(),
        26 => "setGradientV".to_string(),
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fetch(pub [u32; 3]);

impl Fetch {
    pub fn opcode(&self) -> u32 {
        bits(self.0[0], 0, 5)
    }
    pub fn is_vertex(&self) -> bool {
        self.opcode() == 0
    }
    pub fn src_reg(&self) -> u32 {
        bits(self.0[0], 5, 6)
    }
    pub fn src_relative(&self) -> bool {
        bits(self.0[0], 11, 1) == 1
    }
    pub fn dst_reg(&self) -> u32 {
        bits(self.0[0], 12, 6)
    }
    pub fn dst_relative(&self) -> bool {
        bits(self.0[0], 18, 1) == 1
    }
    pub fn dst_swizzle(&self) -> u32 {
        bits(self.0[1], 0, 12)
    }
    pub fn predicated(&self) -> Option<bool> {
        (bits(self.0[1], 31, 1) == 1).then_some(bits(self.0[2], 31, 1) == 1)
    }
    pub fn const_index(&self) -> u32 {
        if self.is_vertex() { bits(self.0[0], 20, 5) * 3 + bits(self.0[0], 25, 2) } else { bits(self.0[0], 20, 5) }
    }
    pub fn tex_src_swizzle(&self) -> u32 {
        bits(self.0[0], 26, 6)
    }
    pub fn tex_dimension(&self) -> u32 {
        bits(self.0[2], 14, 2)
    }
    pub fn vtx_is_mini(&self) -> bool {
        bits(self.0[1], 30, 1) == 1
    }
    pub fn vtx_format(&self) -> u32 {
        bits(self.0[1], 16, 6)
    }
    pub fn vtx_stride(&self) -> u32 {
        bits(self.0[2], 0, 8)
    }
    pub fn vtx_offset(&self) -> i32 {
        sbits(self.0[2], 8, 23)
    }

    fn dst_text(&self) -> String {
        let mut s = format!("r{}", self.dst_reg());
        if self.dst_relative() {
            s = format!("r[{}+aL]", self.dst_reg());
        }
        let sw = self.dst_swizzle();
        if sw != 0b011_010_001_000 {
            s.push('.');
            for c in 0..4 {
                s.push(match (sw >> (3 * c)) & 7 {
                    v @ 0..=3 => COMPONENTS[v as usize],
                    4 => '0',
                    5 => '1',
                    7 => '_',
                    _ => '?',
                });
            }
        }
        s
    }

    pub fn disassemble(&self, tex_name: &dyn Fn(u32) -> Option<String>) -> String {
        let pred = match self.predicated() {
            Some(true) => "(p0) ",
            Some(false) => "(!p0) ",
            None => "",
        };
        let src =
            if self.src_relative() { format!("r[{}+aL]", self.src_reg()) } else { format!("r{}", self.src_reg()) };
        if self.is_vertex() {
            let d = self.0;
            let mini = self.vtx_is_mini();
            let mut s = format!("{pred}{} {}", if mini { "vfetch_mini" } else { "vfetch_full" }, self.dst_text());
            if !mini {
                let sw = COMPONENTS[bits(d[0], 30, 2) as usize];
                s += &format!(", {src}.{sw}, vf{} (const {})", 95 - self.const_index() as i64, self.const_index());
            }
            s += &format!(", Offset={}, DataFormat={}", self.vtx_offset(), self.vtx_format());
            if !mini {
                s += &format!(", Stride={}", self.vtx_stride());
            }
            if bits(d[1], 12, 1) == 1 {
                s += ", Signed=true";
            }
            if bits(d[1], 13, 1) == 1 {
                s += ", NumFormat=integer";
            }
            let exp = sbits(d[1], 24, 6);
            if exp != 0 {
                s += &format!(", ExpAdjust={exp}");
            }
            return s;
        }
        let dim = self.tex_dimension();
        let Some(name) = texture_fetch_name(self.opcode(), dim) else {
            return format!(
                "{pred}fetch_opcode_{} {:08x} {:08x} {:08x}",
                self.opcode(),
                self.0[0],
                self.0[1],
                self.0[2]
            );
        };
        let ncomp = match self.opcode() {
            24 => 1,
            25 | 26 => 3,
            18 => 2,
            _ => [1usize, 2, 3, 3][dim as usize],
        };
        let sw = self.tex_src_swizzle();
        let src_sw: String = (0..ncomp).map(|i| COMPONENTS[((sw >> (2 * i)) & 3) as usize]).collect();
        let has_dst = !matches!(self.opcode(), 24..=26);
        let mut s = format!("{pred}{name} ");
        if has_dst {
            s += &format!("{}, ", self.dst_text());
        }
        s += &format!("{src}.{src_sw}");
        if has_dst {
            let tf = self.const_index();
            s += &format!(", tf{tf}");
            if let Some(n) = tex_name(tf) {
                s += &format!(" ({n})");
            }
        }
        s += &self.tex_attributes();
        s
    }

    fn tex_attributes(&self) -> String {
        const FILTER: [&str; 4] = ["point", "linear", "basemap", "keep"];
        const ANISO: [&str; 8] = ["disabled", "max1to1", "max2to1", "max4to1", "max8to1", "max16to1", "?6", "keep"];
        let d = self.0;
        let mut s = String::new();
        if bits(d[0], 19, 1) == 0 {
            s += ", FetchValidOnly=false";
        }
        if bits(d[0], 25, 1) == 1 {
            s += ", UnnormalizedTextureCoords=true";
        }
        for (label, lo) in
            [("MagFilter", 12), ("MinFilter", 14), ("MipFilter", 16), ("VolMagFilter", 24), ("VolMinFilter", 26)]
        {
            let f = bits(d[1], lo, 2);
            if f != 3 {
                s += &format!(", {label}={}", FILTER[f as usize]);
            }
        }
        let a = bits(d[1], 18, 3);
        if a != 7 {
            s += &format!(", AnisoFilter={}", ANISO[a as usize]);
        }
        if bits(d[1], 28, 1) == 0 {
            s += ", UseComputedLOD=false";
        }
        if bits(d[1], 29, 1) == 1 {
            s += ", UseRegisterLOD=true";
        }
        if bits(d[2], 0, 1) == 1 {
            s += ", UseRegisterGradients=true";
        }
        let bias = sbits(d[2], 2, 7);
        if bias != 0 {
            s += &format!(", LODBias={}", bias as f32 / 16.0);
        }
        for (label, lo) in [("OffsetX", 16), ("OffsetY", 21), ("OffsetZ", 26)] {
            let o = sbits(d[2], lo, 5);
            if o != 0 {
                s += &format!(", {label}={}", o as f32 * 0.5);
            }
        }
        s
    }
}
