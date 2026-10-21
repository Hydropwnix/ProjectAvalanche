pub const CHAPTER_GEN_COUNTS: [(u32, u32); 9] =
    [(2, 4), (3, 3), (4, 3), (5, 3), (6, 3), (7, 3), (8, 3), (9, 2), (10, 3)];

pub const CHAPTERS: u32 = 11;

pub fn chapter_drops(chapter: u32) -> Vec<String> {
    match CHAPTER_GEN_COUNTS.iter().find(|(c, _)| *c == chapter) {
        Some(&(c, n)) => {
            let mut v = vec![format!("chapter_{c}_drop_h2h")];
            v.extend((1..=n).map(|k| format!("chapter_{c}_drop_gen_{k}")));
            v.push(format!("chapter_{c}_drop_dd"));
            v
        }
        None if chapter == 1 || chapter == CHAPTERS => vec![format!("chapter_{chapter}_drop_h2h")],
        None => Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovieWhen {
    Success,
    Failure,
    Unlocked,
    Any,
}

pub const MOVIES: [(&str, &str, MovieWhen); 14] = [
    ("comic_zoe", "chapter_2_drop_dd", MovieWhen::Success),
    ("comic_elise", "chapter_3_drop_h2h", MovieWhen::Success),
    ("comic_alex", "chapter_4_drop_h2h", MovieWhen::Success),
    ("comic_tane", "chapter_5_drop_h2h", MovieWhen::Success),
    ("comic_moby", "chapter_6_drop_h2h", MovieWhen::Success),
    ("comic_kaori", "chapter_7_drop_h2h", MovieWhen::Success),
    ("comic_psymon", "chapter_8_drop_h2h", MovieWhen::Success),
    ("comic_mac", "chapter_9_drop_h2h", MovieWhen::Success),
    ("comic_ty", "chapter_10_drop_h2h", MovieWhen::Success),
    ("endcredits", "chapter_10_drop_dd", MovieWhen::Success),
    ("campaignwin", "chapter_11_drop_h2h", MovieWhen::Success),
    ("griff1", "chapter_5_drop_h2h", MovieWhen::Unlocked),
    ("griff2", "chapter_8_drop_h2h", MovieWhen::Unlocked),
    ("griff3", "chapter_10_drop_h2h", MovieWhen::Unlocked),
];

pub const NEW_GAME_MOVIES: [&str; 2] = ["mainintro", "rockies"];

pub const FIRST_DROP: &str = "chapter_2_drop_h2h";

pub const SECOND_DROP: &str = "chapter_2_drop_gen_1";

pub const PROLOGUE_DROP: &str = "chapter_1_drop_h2h";

pub const NEW_GAME_RIDER: &str = "zoe";
pub const NEW_GAME_ITEMS: [u32; 3] = [0x55BB_536F, 0x8650_5C81, 0xE318_0443];

pub const START_RANGE: &str = "rockies";

pub const MENUS_DROP: &str = "chapter_2_drop_dd";

pub const FINAL_DROP: &str = "chapter_11_drop_h2h";

pub const DEADLY_DESCENTS: [&str; 9] = [
    "chapter_2_drop_dd",
    "chapter_3_drop_dd",
    "chapter_4_drop_dd",
    "chapter_5_drop_dd",
    "chapter_6_drop_dd",
    "chapter_7_drop_dd",
    "chapter_8_drop_dd",
    "chapter_9_drop_dd",
    "chapter_10_drop_dd",
];
