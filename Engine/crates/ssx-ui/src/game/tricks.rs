use super::loc::Localization;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrickRecordView {
    pub kind: i32,
    pub trick: u32,
    pub flips_fwd: u32,
    pub flips_bwd: u32,
    pub interaction: i32,
    pub tier: u32,
    pub tweaked: bool,
    pub awesome: bool,
    pub super_awesome: bool,
    pub nose_press: bool,
    pub goofy: bool,
    pub duplicate: bool,
    pub spin_fs: f32,
    pub spin_bs: f32,
    pub distance: f32,
    pub time: f32,
}

pub fn trick_type_name(loc: &mut Localization, trick: u32, awesome: bool, super_awesome: bool, rider: &str) -> String {
    const GRABS: [&str; 12] = [
        "FLYINGSQUIRREL",
        "INDY",
        "MELON",
        "MUTE",
        "NOSE",
        "ROCKET",
        "SEATBELT",
        "STALEFISH",
        "STIFFY",
        "TAIL",
        "TRUCKDRIVER",
        "WHEELMAN",
    ];
    let key = match trick {
        1 => format!("TXT_SSX_TRICK_NAME_SIGNATURE_{}", rider.to_ascii_uppercase()),
        2..=13 => {
            let g = GRABS[(trick - 2) as usize];
            match (super_awesome, awesome) {
                (true, _) => format!("TXT_SSX_TRICK_TYPE_GRAB_{g}_SUPERUBER"),
                (false, true) => format!("TXT_SSX_TRICK_TYPE_GRAB_{g}_UBER"),
                _ => format!("TXT_SSX_TRICK_TYPE_GRAB_{g}"),
            }
        }
        14..=31 => {
            let n = (trick - 14) / 3 + 1;
            match (trick - 14) % 3 {
                0 => format!("TXT_SSX_TRICK_TYPE_GRIND_{n}"),
                1 => format!("TXT_SSX_TRICK_TYPE_GRIND_{n}_UBER"),
                _ => format!("TXT_SSX_TRICK_TYPE_GRIND_{n}_SUPERUBER"),
            }
        }
        _ => match added_trick_key(loc, trick, awesome, super_awesome) {
            Some(key) => key,
            None => return String::new(),
        },
    };
    loc.text(&key)
}

pub fn trick_name(loc: &mut Localization, r: &TrickRecordView, rider: &str, short: bool) -> String {
    let number = |loc: &Localization, v: f32| loc.number(f64::from(v), 0);
    let mut out = String::new();
    let sep = |out: &mut String| {
        if !out.is_empty() {
            out.push(' ');
        }
    };
    match r.kind {
        4 => out.push_str(&loc.text("TXT_SSX_TRICK_INTERACTION_REWIND")),
        3 => match r.interaction {
            1 => out.push_str(&loc.text("TXT_SSX_TRICK_INTERACTION_HELI")),
            2 => out.push_str(&loc.text("TXT_SSX_TRICK_INTERACTION_FLARE")),
            0 => out.push_str(&loc.text("TXT_SSX_TRICK_INTERACTION_BONK")),
            i if (0..4).contains(&i) => {
                let t = loc.text("TXT_SSX_TRICK_INTERACTION_UPHILL");
                out.push_str(&format!("{t} ({})", number(loc, r.time)));
            }
            _ => {}
        },
        0 => {
            if r.spin_fs > 0.0 {
                out.push_str(&loc.text("TXT_SSX_TRICK_SPIN_FS"));
                out.push(' ');
                out.push_str(&number(loc, r.spin_fs));
            }
            if r.spin_bs > 0.0 {
                sep(&mut out);
                out.push_str(&loc.text("TXT_SSX_TRICK_SPIN_BS"));
                out.push(' ');
                out.push_str(&number(loc, r.spin_bs));
            }
            if r.trick != 0 {
                let name = trick_type_name(loc, r.trick, r.awesome, r.super_awesome, rider);
                sep(&mut out);
                if r.goofy && r.trick != 1 {
                    out.push_str(&loc.text("TXT_SSX_TRICK_SWITCH"));
                    out.push(' ');
                }
                out.push_str(&name);
                if !short && r.tweaked && r.trick != 1 {
                    out.push(' ');
                    out.push_str(&loc.text("TXT_SSX_TRICK_TWEAK"));
                }
            }
            for (n, key) in [(r.flips_fwd, "TXT_SSX_TRICK_FLIP_FWD"), (r.flips_bwd, "TXT_SSX_TRICK_FLIP_BWD")] {
                if n > 0 {
                    sep(&mut out);
                    let flip = loc.text(if short { "TXT_SSX_TRICK_FLIP" } else { key });
                    if n == 1 {
                        out.push_str(&flip);
                    } else {
                        out.push_str(&format!("{flip} x{}", loc.int(i64::from(n))));
                    }
                }
            }
        }
        1 => {
            out.push_str(&loc.distance_with_unit(f64::from(r.distance)));
            out.push(' ');
            if r.spin_fs > 0.0 {
                out.push_str(&loc.text("TXT_SSX_TRICK_SPIN_FS"));
                out.push(' ');
                out.push_str(&number(loc, r.spin_fs));
                out.push(' ');
            }
            if r.spin_bs > 0.0 {
                out.push_str(&loc.text("TXT_SSX_TRICK_SPIN_BS"));
                out.push(' ');
                out.push_str(&number(loc, r.spin_bs));
                out.push(' ');
            }
            if r.trick != 0 {
                out.push_str(&trick_type_name(loc, r.trick, r.awesome, r.super_awesome, rider));
                out.push(' ');
            }
            out.push_str(&loc.text(if r.tweaked { "TXT_SSX_TRICK_BOARDSLIDE" } else { "TXT_SSX_TRICK_GRIND" }));
        }
        2 => {
            out.push_str(&loc.distance_with_unit(f64::from(r.distance)));
            out.push(' ');
            out.push_str(&loc.text(if r.nose_press { "TXT_SSX_TRICK_NOSEPRESS" } else { "TXT_SSX_TRICK_TAILPRESS" }));
        }
        _ => {}
    }
    out
}

fn added_trick_key(loc: &Localization, trick: u32, awesome: bool, super_awesome: bool) -> Option<String> {
    let routes = loc.content.as_ref()?.trick_routes();
    let t = routes.by_value(trick)?;
    let level = usize::from(awesome || super_awesome) + usize::from(super_awesome);
    t.trick.names.get(level).map(ToString::to_string)
}
