use icu_properties::CodePointMapData;
use icu_properties::props::{BidiClass, BidiMirroringGlyph, BidiPairedBracketType};

use super::{Bracket, Class};
use crate::text::isolate;

pub fn of(c: char) -> Class {
    match c {
        isolate::OPEN => return Class::FSI,
        isolate::CLOSE => return Class::PDI,
        _ => {}
    }
    match CodePointMapData::<BidiClass>::new().get(c) {
        BidiClass::RightToLeft => Class::R,
        BidiClass::ArabicLetter => Class::AL,
        BidiClass::EuropeanNumber => Class::EN,
        BidiClass::EuropeanSeparator => Class::ES,
        BidiClass::EuropeanTerminator => Class::ET,
        BidiClass::ArabicNumber => Class::AN,
        BidiClass::CommonSeparator => Class::CS,
        BidiClass::NonspacingMark => Class::NSM,
        BidiClass::BoundaryNeutral => Class::BN,
        BidiClass::ParagraphSeparator => Class::B,
        BidiClass::SegmentSeparator => Class::S,
        BidiClass::WhiteSpace => Class::WS,
        BidiClass::OtherNeutral => Class::ON,
        BidiClass::LeftToRightEmbedding => Class::LRE,
        BidiClass::LeftToRightOverride => Class::LRO,
        BidiClass::RightToLeftEmbedding => Class::RLE,
        BidiClass::RightToLeftOverride => Class::RLO,
        BidiClass::PopDirectionalFormat => Class::PDF,
        BidiClass::FirstStrongIsolate => Class::FSI,
        BidiClass::LeftToRightIsolate => Class::LRI,
        BidiClass::RightToLeftIsolate => Class::RLI,
        BidiClass::PopDirectionalIsolate => Class::PDI,
        _ => Class::L,
    }
}

fn canonical(c: char) -> u32 {
    match c {
        '\u{2329}' => 0x3008,
        '\u{232A}' => 0x3009,
        c => u32::from(c),
    }
}

pub fn bracket(c: char) -> Bracket {
    let m = CodePointMapData::<BidiMirroringGlyph>::new().get(c);
    match m.paired_bracket_type {
        BidiPairedBracketType::Open => Bracket::Open(canonical(c)),
        BidiPairedBracketType::Close => match m.mirroring_glyph {
            Some(open) => Bracket::Close(canonical(open)),
            None => Bracket::None,
        },
        _ => Bracket::None,
    }
}

pub fn mirror(c: char) -> Option<char> {
    let m = CodePointMapData::<BidiMirroringGlyph>::new().get(c);
    if m.mirrored { m.mirroring_glyph } else { None }
}
