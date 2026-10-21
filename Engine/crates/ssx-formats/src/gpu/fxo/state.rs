#[rustfmt::skip]
pub const RENDER_STATES: [(u16, &str, u32); 90] = [
    (0x028, "ZENABLE", 1), (0x034, "FILLMODE", 0), (0x030, "ZWRITEENABLE", 1),
    (0x060, "ALPHATESTENABLE", 0), (0x048, "SRCBLEND", 1), (0x04C, "DESTBLEND", 0),
    (0x038, "CULLMODE", 6), (0x02C, "ZFUNC", 3), (0x064, "ALPHAREF", 0), (0x068, "ALPHAFUNC", 7),
    (0x03C, "ALPHABLENDENABLE", 0), (0x06C, "STENCILENABLE", 0), (0x074, "STENCILFAIL", 0),
    (0x078, "STENCILZFAIL", 0), (0x07C, "STENCILPASS", 0), (0x080, "STENCILFUNC", 7),
    (0x084, "STENCILREF", 0), (0x088, "STENCILMASK", 0xFFFF_FFFF), (0x08C, "STENCILWRITEMASK", 0xFFFF_FFFF),
    (0x0F0, "WRAP0", 0), (0x0F4, "WRAP1", 0), (0x0F8, "WRAP2", 0), (0x0FC, "WRAP3", 0),
    (0x100, "WRAP4", 0), (0x104, "WRAP5", 0), (0x108, "WRAP6", 0), (0x10C, "WRAP7", 0),
    (0x0AC, "CLIPPLANEENABLE", 0), (0x0B0, "POINTSIZE", 0x3F80_0000), (0x0B4, "POINTSIZE_MIN", 0x3F80_0000),
    (0x0B8, "POINTSPRITEENABLE", 0), (0x0C0, "MULTISAMPLEANTIALIAS", 1), (0x0C4, "MULTISAMPLEMASK", 0xFFFF_FFFF),
    (0x0BC, "POINTSIZE_MAX", 0x4280_0000), (0x0D4, "COLORWRITEENABLE", 0xF), (0x050, "BLENDOP", 0),
    (0x0C8, "SCISSORTESTENABLE", 0), (0x0CC, "SLOPESCALEDEPTHBIAS", 0),
    (0x0E8, "MINTESSELLATIONLEVEL", 0x3F80_0000), (0x0EC, "MAXTESSELLATIONLEVEL", 0x3F80_0000),
    (0x070, "TWOSIDEDSTENCILMODE", 0), (0x090, "CCW_STENCILFAIL", 0), (0x094, "CCW_STENCILZFAIL", 0),
    (0x098, "CCW_STENCILPASS", 0), (0x09C, "CCW_STENCILFUNC", 7), (0x0A0, "CCW_STENCILREF", 0),
    (0x0A4, "CCW_STENCILMASK", 0xFFFF_FFFF), (0x0A8, "CCW_STENCILWRITEMASK", 0xFFFF_FFFF),
    (0x0D8, "COLORWRITEENABLE1", 0xF), (0x0DC, "COLORWRITEENABLE2", 0xF), (0x0E0, "COLORWRITEENABLE3", 0xF),
    (0x044, "BLENDFACTOR", 0xFFFF_FFFF), (0x0D0, "DEPTHBIAS", 0),
    (0x110, "WRAP8", 0), (0x114, "WRAP9", 0), (0x118, "WRAP10", 0), (0x11C, "WRAP11", 0),
    (0x120, "WRAP12", 0), (0x124, "WRAP13", 0), (0x128, "WRAP14", 0), (0x12C, "WRAP15", 0),
    (0x040, "SEPARATEALPHABLENDENABLE", 0), (0x054, "SRCBLENDALPHA", 1), (0x058, "DESTBLENDALPHA", 0),
    (0x05C, "BLENDOPALPHA", 0), (0x130, "VIEWPORTENABLE", 1), (0x134, "HIGHPRECISIONBLENDENABLE", 0),
    (0x138, "HIGHPRECISIONBLENDENABLE1", 0), (0x13C, "HIGHPRECISIONBLENDENABLE2", 0),
    (0x140, "HIGHPRECISIONBLENDENABLE3", 0), (0x144, "HALFPIXELOFFSET", 0), (0x0E4, "TESSELLATIONMODE", 1),
    (0x148, "PRIMITIVERESETENABLE", 0), (0x14C, "PRIMITIVERESETINDEX", 0xFFFF),
    (0x150, "ALPHATOMASKENABLE", 0), (0x154, "ALPHATOMASKOFFSETS", 0x87),
    (0x158, "GUARDBAND_X", 0x4000_0000), (0x15C, "GUARDBAND_Y", 0x4000_0000),
    (0x160, "DISCARDBAND_X", 0x3F80_0000), (0x164, "DISCARDBAND_Y", 0x3F80_0000),
    (0x168, "HISTENCILENABLE", 0), (0x16C, "HISTENCILWRITEENABLE", 0), (0x170, "HISTENCILFUNC", 0),
    (0x174, "HISTENCILREF", 0), (0x178, "PRESENTINTERVAL", 0), (0x17C, "PRESENTIMMEDIATETHRESHOLD", 0),
    (0x180, "HIZENABLE", 2), (0x184, "HIZWRITEENABLE", 2), (0x188, "LASTPIXEL", 1),
    (0x18C, "LINEWIDTH", 0x3F80_0000),
];

#[rustfmt::skip]
pub const SAMPLER_STATES: [(u16, &str, u32); 20] = [
    (0x00, "ADDRESSU", 0), (0x04, "ADDRESSV", 0), (0x08, "ADDRESSW", 0), (0x0C, "BORDERCOLOR", 0),
    (0x10, "MAGFILTER", 0), (0x14, "MINFILTER", 0), (0x18, "MIPFILTER", 2), (0x1C, "MIPMAPLODBIAS", 0),
    (0x20, "MAXMIPLEVEL", 0), (0x24, "MAXANISOTROPY", 1), (0x28, "MAGFILTERZ", 0), (0x2C, "MINFILTERZ", 0),
    (0x30, "SEPARATEZFILTERENABLE", 0), (0x34, "MINMIPLEVEL", 13), (0x38, "TRILINEARTHRESHOLD", 0),
    (0x3C, "ANISOTROPYBIAS", 0), (0x40, "HGRADIENTEXPBIAS", 0), (0x44, "VGRADIENTEXPBIAS", 0),
    (0x48, "WHITEBORDERCOLORW", 0), (0x4C, "POINTBORDERENABLE", 1),
];

pub const ZENABLE: u16 = 0x028;
pub const ZFUNC: u16 = 0x02C;
pub const ZWRITEENABLE: u16 = 0x030;
pub const CULLMODE: u16 = 0x038;
pub const ALPHABLENDENABLE: u16 = 0x03C;
pub const SRCBLEND: u16 = 0x048;
pub const DESTBLEND: u16 = 0x04C;
pub const BLENDOP: u16 = 0x050;
pub const ALPHATESTENABLE: u16 = 0x060;
pub const ALPHAREF: u16 = 0x064;
pub const ALPHAFUNC: u16 = 0x068;
pub const STENCILENABLE: u16 = 0x06C;
pub const STENCILPASS: u16 = 0x07C;
pub const STENCILFUNC: u16 = 0x080;
pub const STENCILREF: u16 = 0x084;
pub const STENCILMASK: u16 = 0x088;
pub const COLORWRITEENABLE: u16 = 0x0D4;
pub const ADDRESSU: u16 = 0x00;
pub const ADDRESSV: u16 = 0x04;
pub const ADDRESSW: u16 = 0x08;
pub const MAGFILTER: u16 = 0x10;
pub const MINFILTER: u16 = 0x14;
pub const MIPFILTER: u16 = 0x18;
pub const MIPMAPLODBIAS: u16 = 0x1C;
pub const MAXMIPLEVEL: u16 = 0x20;
pub const MAXANISOTROPY: u16 = 0x24;

pub const CULL_NONE: u32 = 0;
pub const CULL_CW: u32 = 2;
pub const CULL_CCW: u32 = 6;
pub const CMP_NAMES: [&str; 8] =
    ["NEVER", "LESS", "EQUAL", "LESSEQUAL", "GREATER", "NOTEQUAL", "GREATEREQUAL", "ALWAYS"];
pub const ADDRESS_NAMES: [&str; 8] =
    ["WRAP", "MIRROR", "CLAMP", "MIRRORONCE", "BORDER_HALF", "MIRRORONCE_BORDER_HALF", "BORDER", "MIRRORONCE_BORDER"];
pub const FILTER_POINT: u32 = 0;
pub const FILTER_LINEAR: u32 = 1;
pub const FILTER_NONE: u32 = 2;
pub const FILTER_ANISOTROPIC: u32 = 4;

pub fn blend_name(v: u32) -> Option<&'static str> {
    Some(match v {
        0 => "ZERO",
        1 => "ONE",
        4 => "SRCCOLOR",
        5 => "INVSRCCOLOR",
        6 => "SRCALPHA",
        7 => "INVSRCALPHA",
        8 => "DESTCOLOR",
        9 => "INVDESTCOLOR",
        10 => "DESTALPHA",
        11 => "INVDESTALPHA",
        12 => "BLENDFACTOR",
        13 => "INVBLENDFACTOR",
        16 => "SRCALPHASAT",
        _ => return None,
    })
}

pub fn render_state_name(state: u16) -> Option<&'static str> {
    RENDER_STATES.iter().find(|(s, _, _)| *s == state).map(|(_, n, _)| *n)
}

pub fn render_state_default(state: u16) -> Option<u32> {
    RENDER_STATES.iter().find(|(s, _, _)| *s == state).map(|(_, _, d)| *d)
}

pub fn sampler_state_name(state: u16) -> Option<&'static str> {
    SAMPLER_STATES.iter().find(|(s, _, _)| *s == state).map(|(_, n, _)| *n)
}

pub fn sampler_state_default(state: u16) -> Option<u32> {
    SAMPLER_STATES.iter().find(|(s, _, _)| *s == state).map(|(_, _, d)| *d)
}

pub fn value_name(sampler_block: bool, state: u16, v: u32) -> Option<&'static str> {
    const BOOL: [&str; 2] = ["FALSE", "TRUE"];
    const STENCIL_OP: [&str; 8] = ["KEEP", "ZERO", "REPLACE", "INCRSAT", "DECRSAT", "INVERT", "INCR", "DECR"];
    const FILTER: [&str; 5] = ["POINT", "LINEAR", "NONE", "?", "ANISOTROPIC"];
    let i = v as usize;
    if sampler_block {
        return match state {
            0x00 | 0x04 | 0x08 => ADDRESS_NAMES.get(i).copied(),
            0x10 | 0x14 | 0x18 | 0x28 | 0x2C => FILTER.get(i).copied().filter(|n| *n != "?"),
            0x30 | 0x48 | 0x4C => BOOL.get(i).copied(),
            _ => None,
        };
    }
    match state {
        0x028 | 0x030 | 0x03C | 0x040 | 0x060 | 0x06C | 0x070 | 0x168 | 0x16C => BOOL.get(i).copied(),
        0x02C | 0x068 | 0x080 | 0x09C => CMP_NAMES.get(i).copied(),
        0x038 => match v {
            CULL_NONE => Some("NONE"),
            CULL_CW => Some("CW"),
            CULL_CCW => Some("CCW"),
            _ => None,
        },
        0x048 | 0x04C | 0x054 | 0x058 => blend_name(v),
        0x050 | 0x05C => ["ADD", "SUBTRACT", "MIN", "MAX", "REVSUBTRACT"].get(i).copied(),
        0x074 | 0x078 | 0x07C | 0x090 | 0x094 | 0x098 => STENCIL_OP.get(i).copied(),
        _ => None,
    }
}
