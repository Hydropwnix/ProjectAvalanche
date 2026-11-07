#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub scale: [f32; 3],
    pub rotation: [f32; 4],
    pub position: [f32; 3],
    pub unk: [f32; 6],
}

impl Placement {
    pub fn ao_multiplier(&self) -> f32 {
        self.unk[2]
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 64 {
            return None;
        }
        let f = |i: usize| f32::from_be_bytes(bytes[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
        Some(Placement {
            scale: [f(0), f(1), f(2)],
            rotation: [f(4), f(5), f(6), f(7)],
            position: [f(8), f(9), f(10)],
            unk: [f(3), f(11), f(12), f(13), f(14), f(15)],
        })
    }
}
