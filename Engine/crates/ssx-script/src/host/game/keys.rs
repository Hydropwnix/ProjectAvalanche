pub const BINDABLE: &[&str] = &[
    "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f10", "f11", "f12", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    "num_0", "num_1", "num_2", "num_3", "num_4", "num_5", "num_6", "num_7", "num_8", "num_9", "num_add", "num_sub",
    "num_mul", "num_div", "num_dec", "b", "m", "o", "p", "u", ",", ".", "/", ";", "'", "[", "]", "-", "=", "`", "\\",
    "insert", "del", "pause", "scroll", "up", "down", "left", "right", "enter", "bksp", "dp_up", "dp_down", "dp_left",
    "dp_right", "pad_a", "pad_b", "pad_x", "pad_y", "pad_lb", "pad_rb", "pad_view", "pad_menu", "pad_ls", "pad_rs",
    "a", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "n", "q", "r", "s", "t", "v", "w", "x", "y", "z", "space",
    "mouse1", "mouse2", "mouse3", "wheel_up", "wheel_dn",
];

pub const STATES: &[&str] = &["down", "up", "both"];

pub fn bindable(name: &str) -> Option<&'static str> {
    BINDABLE.iter().copied().find(|k| *k == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_unique_and_short_names() {
        let mut seen = std::collections::BTreeSet::new();
        for k in BINDABLE {
            assert!(seen.insert(*k), "{k} twice");
            assert!(!k.is_empty() && k.len() <= 8 && k.is_ascii(), "{k:?}");
            assert_ne!(*k, "f9", "the monitor's key");
            assert_ne!(*k, "f1", "the movie skip");
        }
    }
}
