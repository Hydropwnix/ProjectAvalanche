#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub key: u32,
    pub name: String,
    pub title: String,
    pub movie: String,
    pub unk_index: i64,
    pub selectable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Peak {
    pub key: u32,
    pub name: String,
    pub range: u32,
    pub title: String,
    pub info: String,
    pub image: String,
    pub selectable: bool,
    pub campaign_type: i64,
    pub unk_flag: bool,
    pub unk_floats: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drop {
    pub key: u32,
    pub track: String,
    pub track_key: u32,
    pub peak: String,
    pub title: String,
    pub selectable: bool,
    pub difficulty: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DropType {
    Explore,
    Campaign,
    Type2,
    Custom,
    None,
}

impl DropType {
    pub fn from_raw(v: i64) -> Self {
        match v {
            0 => DropType::Explore,
            1 => DropType::Campaign,
            2 => DropType::Type2,
            3 => DropType::Custom,
            _ => DropType::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub key: u32,
    pub name: String,
    pub drop: u32,
    pub event_type: i64,
    pub drop_type: DropType,
    pub mode: String,
    pub slot: String,
    pub peak_short: String,
    pub hud_title: String,
    pub hud_body: String,
    pub goal_body: String,
    pub results_title: String,
    pub results_body: String,
    pub unlock_text: String,
    pub deadly_descent: bool,
    pub warning: i64,
    pub time_of_day: i64,
    pub cost: i64,
    pub difficulty: i64,
    pub reward: i64,
    pub places: Option<i64>,
    pub targets: Vec<i64>,
    pub required: Vec<i64>,
    pub unk_b683b553: Vec<i64>,
    pub rivals: Vec<u32>,
    pub forced: Vec<u32>,
    pub unk_63180fcc: bool,
    pub unk_89d02fa4: bool,
    pub unk_12bccfb5: i64,
    pub unk_0b03fc19: Option<i64>,
    pub unk_98e9ebcc: Vec<i64>,
    pub custom_status: i64,
    pub unk_links: [u32; 3],
    pub peak_ref: u32,
    pub helicopter: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rider {
    pub key: u32,
    pub name: String,
    pub title: String,
    pub display_name: String,
    pub cost: i64,
    pub locked: bool,
    pub default: bool,
    pub unk_940b9b22: i64,
    pub stat_bonuses: Vec<(i32, u8)>,
    pub portrait: String,
    pub description: String,
    pub round_icon: String,
    pub icon: String,
    pub content: Option<RiderContent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiderContent {
    pub id: ssx_content::ContentId,
    pub stand_in: u32,
    pub portrait: Option<ssx_content::ContentId>,
    pub round_icon: Option<ssx_content::ContentId>,
    pub icon: Option<ssx_content::ContentId>,
    pub added: bool,
    pub default_board: Option<u32>,
    pub disc_stat_bonuses: Option<Vec<(i32, u8)>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemContent {
    pub id: ssx_content::ContentId,
    pub added: bool,
    pub disc_attributes: Option<Vec<(i32, u8)>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: u32,
    pub group: u32,
    pub title: String,
    pub kind: i64,
    pub slot: i64,
    pub rider: u32,
    pub locked: bool,
    pub default: bool,
    pub level: i64,
    pub quality: i64,
    pub cost: i64,
    pub attributes: Vec<(i32, u8)>,
    pub unk_b6ef00c6: i64,
    pub descriptions: Vec<u32>,
    pub content: Option<ItemContent>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoreTemplate {
    pub key: u32,
    pub title: String,
    pub slot: i64,
    pub kind: i64,
    pub stats: Vec<i64>,
    pub price_scale: f32,
    pub charlock_chance: i64,
    pub bonus_chance: i64,
    pub min_level: i64,
    pub max_level: i64,
    pub min_rarity: i64,
    pub max_rarity: i64,
    pub fixed_points: i64,
    pub bonus_stats: Vec<i64>,
    pub descriptions: Vec<u32>,
    pub members: Vec<u32>,
}

pub const STORE_INFO_KEYS: [u32; 21] = [
    0x1A3F_D5C3,
    0x98AF_33F3,
    0xCBD9_D504,
    0x01F0_1ABE,
    0x1FEF_6756,
    0xE27A_B77B,
    0xA1A6_A9D3,
    0x59E3_CBF1,
    0x6E97_A28C,
    0x7B55_AC0D,
    0x48D8_3C1E,
    0x8129_F431,
    0x1E82_2913,
    0x0985_4FC3,
    0xD059_1403,
    0xAAC3_9713,
    0xF9C9_A6B2,
    0x1182_F749,
    0xFC57_3105,
    0x4464_A727,
    0x0667_1AD9,
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Progression {
    pub level_thresholds: Vec<i64>,
    pub level_prices: Vec<i64>,
    pub level_slots: Vec<[u8; 5]>,
    pub unk: Vec<(u32, Vec<i64>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExploreRewards {
    pub tier: String,
    pub bronze: i64,
    pub silver: i64,
    pub gold: i64,
    pub per_place_passed: i64,
    pub unk: Vec<(u32, f64)>,
}
