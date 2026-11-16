// PORT(choice): a presence text is at most 128 bytes of UTF-8 (cut on a character boundary, "…" at the cut) and at
// least 2 characters, else left out. Discord's Game SDK documented 128 bytes for `details`, `state`, `large_text`;
// its API refuses a string under 2 characters; a UTF-8 byte count bounds the UTF-16 count Discord's client may count
// in. The console had no Discord.
pub const TEXT_MAX: usize = 128;
pub const TEXT_MIN: usize = 2;
pub const LABEL_MAX: usize = 32;
pub const URL_MAX: usize = 512;
// PORT(choice): an image is named by an art asset key of the application, 1 to 32 of `a-z 0-9 _ -` (the keys the
// Developer Portal makes; the Game SDK documented 32 bytes); no external image URL is sent. The console had no
// Discord.
pub const KEY_MAX: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityType {
    Playing,
    Competing,
}

impl ActivityType {
    fn number(self) -> u8 {
        match self {
            ActivityType::Playing => 0,
            ActivityType::Competing => 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Text(String);

impl Text {
    pub fn new(s: &str) -> Option<Text> {
        let mut out = String::with_capacity(s.len().min(TEXT_MAX));
        let mut space = false;
        let mut cut = false;
        for c in s.chars() {
            if c.is_whitespace() {
                space = !out.is_empty();
                continue;
            }
            if ssx_core::text::is_hidden(c) {
                continue;
            }
            let needed = c.len_utf8() + usize::from(space);
            if out.len() + needed > TEXT_MAX {
                cut = true;
                break;
            }
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
        if cut {
            const ELLIPSIS: char = '…';
            while out.len() + ELLIPSIS.len_utf8() > TEXT_MAX {
                out.pop();
            }
            let trimmed = out.trim_end().len();
            out.truncate(trimmed);
            out.push(ELLIPSIS);
        }
        (out.chars().count() >= TEXT_MIN).then_some(Text(out))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetKey(String);

impl AssetKey {
    pub fn new(key: &str) -> Option<AssetKey> {
        let ok = (1..=KEY_MAX).contains(&key.len())
            && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        ok.then(|| AssetKey(key.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    label: String,
    url: String,
}

impl Button {
    pub fn new(label: &str, url: &str) -> Option<Button> {
        let label_ok = (1..=LABEL_MAX).contains(&label.chars().count())
            && !label.chars().any(ssx_core::text::is_hidden)
            && label.trim() == label;
        let url_ok = (1..=URL_MAX).contains(&url.len())
            && url.starts_with("https://")
            && url.len() > "https://".len()
            && url.bytes().all(|b| b.is_ascii_graphic());
        (label_ok && url_ok).then(|| Button { label: label.to_string(), url: url.to_string() })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub kind: ActivityType,
    pub details: Option<Text>,
    pub state: Option<Text>,
    pub start: Option<u64>,
    pub end: Option<u64>,
    pub large_image: Option<AssetKey>,
    pub large_text: Option<Text>,
    buttons: Vec<Button>,
}

impl Activity {
    pub fn new(kind: ActivityType) -> Self {
        Activity {
            kind,
            details: None,
            state: None,
            start: None,
            end: None,
            large_image: None,
            large_text: None,
            buttons: Vec::new(),
        }
    }

    pub fn push_button(&mut self, b: Button) -> bool {
        if self.buttons.len() >= 2 {
            return false;
        }
        self.buttons.push(b);
        true
    }

    pub fn buttons(&self) -> &[Button] {
        &self.buttons
    }

    pub fn to_json(&self) -> serde_json::Value {
        let mut a = serde_json::Map::new();
        a.insert("type".into(), self.kind.number().into());
        let mut text = |key: &str, t: &Option<Text>| {
            if let Some(t) = t {
                a.insert(key.into(), t.as_str().into());
            }
        };
        text("details", &self.details);
        text("state", &self.state);
        let mut timestamps = serde_json::Map::new();
        // PORT(choice): Unix seconds, as the IPC's own SET_ACTIVITY example sends (`time(nullptr)`) and the Social
        // SDK documents; the gateway's activity object shows milliseconds, which the client makes of them. The
        // console had no Discord.
        if let Some(s) = self.start {
            timestamps.insert("start".into(), s.into());
        }
        if let Some(e) = self.end {
            timestamps.insert("end".into(), e.into());
        }
        if !timestamps.is_empty() {
            a.insert("timestamps".into(), timestamps.into());
        }
        let mut assets = serde_json::Map::new();
        if let Some(k) = &self.large_image {
            assets.insert("large_image".into(), k.0.as_str().into());
        }
        if let Some(t) = &self.large_text {
            assets.insert("large_text".into(), t.as_str().into());
        }
        if !assets.is_empty() {
            a.insert("assets".into(), assets.into());
        }
        if !self.buttons.is_empty() {
            let buttons = self.buttons.iter().map(|b| serde_json::json!({ "label": b.label, "url": b.url }));
            a.insert("buttons".into(), buttons.collect());
        }
        serde_json::Value::Object(a)
    }
}

#[cfg(test)]
mod tests;
