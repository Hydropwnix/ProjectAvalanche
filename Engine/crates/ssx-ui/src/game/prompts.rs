#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pad {
    A,
    B,
    X,
    Y,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Back,
    Start,
    LeftThumb,
    RightThumb,
    Dpad(i8, i8),
    LeftStick(i8, i8),
    RightStick(i8, i8),
}

pub const TOKENS: [&str; 43] = [
    "NEXT",
    "RS_R_DOWN",
    "RS_L_DOWN",
    "RS_R_L",
    "RS_L_R",
    "RS_UP_L",
    "RS_UP_R",
    "RS_R_UP",
    "RS_L_UP",
    "SELECT",
    "START",
    "R3",
    "L3",
    "DPAD_UD",
    "DPAD_LR",
    "DPAD_RIGHT",
    "DPAD_LEFT",
    "DPAD_DOWN",
    "DPAD_UP",
    "RS_UD",
    "RS_LR",
    "RS_RIGHT",
    "RS_LEFT",
    "RS_DOWN",
    "RS_UP",
    "LS_UD",
    "LS_LR",
    "LS_RIGHT",
    "LS_LEFT",
    "LS_DOWN",
    "LS_UP",
    "RS",
    "LS",
    "Y",
    "X",
    "RT",
    "RB",
    "LT",
    "LB",
    "B",
    "A",
    "ADVANCE",
    "BACK",
];

pub fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(a) = rest.find('^') {
        let Some(len) = rest[a + 1..].find('^') else { break };
        let name = &rest[a + 1..a + 1 + len];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push(name);
            rest = &rest[a + len + 2..];
        } else {
            rest = &rest[a + 1..];
        }
    }
    out
}

pub fn pad(token: &str) -> Option<Pad> {
    Some(match token {
        "A" | "ADVANCE" => Pad::A,
        "B" | "BACK" => Pad::B,
        "X" => Pad::X,
        "Y" => Pad::Y,
        "LB" => Pad::LeftBumper,
        "RB" => Pad::RightBumper,
        "LT" => Pad::LeftTrigger,
        "RT" => Pad::RightTrigger,
        "SELECT" => Pad::Back,
        "START" => Pad::Start,
        "L3" => Pad::LeftThumb,
        "R3" => Pad::RightThumb,
        "DPAD_UP" | "DPAD_UD" => Pad::Dpad(0, 1),
        "DPAD_DOWN" => Pad::Dpad(0, -1),
        "DPAD_LEFT" => Pad::Dpad(-1, 0),
        "DPAD_RIGHT" | "DPAD_LR" => Pad::Dpad(1, 0),
        "LS_UP" | "LS_UD" | "LS" => Pad::LeftStick(0, 1),
        "LS_DOWN" => Pad::LeftStick(0, -1),
        "LS_LEFT" => Pad::LeftStick(-1, 0),
        "LS_RIGHT" | "LS_LR" => Pad::LeftStick(1, 0),
        "RS_UP" | "RS_UD" | "RS" => Pad::RightStick(0, 1),
        "RS_DOWN" => Pad::RightStick(0, -1),
        "RS_LEFT" => Pad::RightStick(-1, 0),
        "RS_RIGHT" | "RS_LR" => Pad::RightStick(1, 0),
        "RS_L_UP" | "RS_R_UP" => Pad::RightStick(0, 1),
        "RS_L_DOWN" | "RS_R_DOWN" => Pad::RightStick(0, -1),
        "RS_UP_L" | "RS_R_L" => Pad::RightStick(-1, 0),
        "RS_UP_R" | "RS_L_R" => Pad::RightStick(1, 0),
        _ => return None,
    })
}

pub fn chords(text: &str) -> Vec<Vec<Pad>> {
    let mut out: Vec<Vec<Pad>> = vec![Vec::new()];
    for t in tokens(text) {
        if t == "NEXT" {
            out.push(Vec::new());
        } else if let (Some(p), Some(chord)) = (pad(t), out.last_mut())
            && !chord.contains(&p)
        {
            chord.push(p);
        }
    }
    out.retain(|c| !c.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_chords_of_the_tutorial_prompts() {
        assert_eq!(tokens("Push ^LS_UP^ to start the run!"), ["LS_UP"]);
        assert_eq!(tokens("no tokens, a lone ^ and 5^2"), Vec::<&str>::new());
        assert_eq!(tokens("^X^, ^Y^, or ^B^ to start air tricks."), ["X", "Y", "B"]);
        assert_eq!(chords("^Y^ HOLD"), [[Pad::Y]]);
        assert_eq!(chords("^X^ ^NEXT^ ^Y^ "), [vec![Pad::X], vec![Pad::Y]]);
        assert_eq!(chords("^X^ + ^RT^ "), [[Pad::X, Pad::RightTrigger]]);
        assert_eq!(chords("^LT^ + ^RT^ + ^Y^"), [[Pad::LeftTrigger, Pad::RightTrigger, Pad::Y]]);
        assert_eq!(chords("^LS_LEFT^ SPIN"), [[Pad::LeftStick(-1, 0)]]);
        assert_eq!(chords(" "), Vec::<Vec<Pad>>::new());
        for (analog, digital, _) in crate::game::tutorial::BUTTONS {
            for t in tokens(analog).into_iter().chain(tokens(digital)) {
                assert!(TOKENS.contains(&t), "{t}");
            }
        }
        assert!(TOKENS.iter().all(|t| *t == "NEXT" || pad(t).is_some()));
    }
}
