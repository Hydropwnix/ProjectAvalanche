use super::*;
use crate::test_rng::Rng;

#[test]
fn texts_drop_hidden_characters_and_fold_white_space() {
    let t = Text::new("  Grand\u{202E}  Golliat\u{200B}\n\tRace\u{0007} It  ").unwrap();
    assert_eq!(t.as_str(), "Grand Golliat Race It");
    assert_eq!(Text::new("a\tb\nc").unwrap().as_str(), "a b c");
    assert_eq!(Text::new("\u{2066}Mont\u{2069} \u{E0041}Blanc\u{FEFF}").unwrap().as_str(), "Mont Blanc");
}

#[test]
fn texts_shorter_than_two_characters_are_left_out() {
    assert_eq!(Text::new(""), None);
    assert_eq!(Text::new("x"), None);
    assert_eq!(Text::new(" \u{200B} x \u{202E}"), None);
    assert_eq!(Text::new("xy").unwrap().as_str(), "xy");
    assert_eq!(Text::new("é"), None);
    assert_eq!(Text::new("雪山").unwrap().as_str(), "雪山");
}

#[test]
fn long_texts_are_cut_on_a_character_boundary_with_an_ellipsis() {
    let t = Text::new(&"a".repeat(500)).unwrap();
    assert_eq!(t.as_str().len(), TEXT_MAX);
    assert!(t.as_str().ends_with('…'));
    let t = Text::new(&"雪".repeat(100)).unwrap();
    assert!(t.as_str().len() <= TEXT_MAX);
    assert!(t.as_str().ends_with('…'));
    let t = Text::new(&"b".repeat(TEXT_MAX)).unwrap();
    assert_eq!(t.as_str(), "b".repeat(TEXT_MAX));
    let t = Text::new(&format!("{} {}", "c".repeat(124), "d".repeat(10))).unwrap();
    assert_eq!(t.as_str(), format!("{}…", "c".repeat(124)));
}

#[test]
fn random_texts_are_always_safe() {
    let mut rng = Rng::new(7);
    let pool: Vec<char> =
        "aZ9 \t\n\u{0}\u{1b}\u{7f}\u{85}\u{200B}\u{200E}\u{202A}\u{202E}\u{2066}\u{2028}\u{FEFF}\u{E0001}é雪😀…"
            .chars()
            .collect();
    for _ in 0..20_000 {
        let len = rng.below(200);
        let s: String = (0..len).map(|_| pool[rng.below(pool.len())]).collect();
        if let Some(t) = Text::new(&s) {
            let t = t.as_str();
            assert!(t.len() <= TEXT_MAX, "{t:?}");
            assert!(t.chars().count() >= TEXT_MIN);
            assert!(!t.chars().any(ssx_core::text::is_hidden), "{t:?}");
            assert_eq!(t.trim(), t);
            assert!(!t.contains("  "));
        }
    }
}

#[test]
fn asset_keys_are_portal_keys() {
    assert!(AssetKey::new("logo").is_some());
    assert!(AssetKey::new("mode_race-1").is_some());
    assert!(AssetKey::new("").is_none());
    assert!(AssetKey::new("Logo").is_none());
    assert!(AssetKey::new("https://example.com/a.png").is_none());
    assert!(AssetKey::new(&"k".repeat(KEY_MAX + 1)).is_none());
}

#[test]
fn buttons_are_https_links_with_short_labels() {
    assert!(Button::new("Join the Discord", "https://discord.com/invite/XrFGQEfwdJ").is_some());
    assert!(Button::new("", "https://example.com").is_none());
    assert!(Button::new(&"l".repeat(LABEL_MAX + 1), "https://example.com").is_none());
    assert!(Button::new("ok", "http://example.com").is_none());
    assert!(Button::new("ok", "https://").is_none());
    assert!(Button::new("ok", "https://exa mple.com").is_none());
    assert!(Button::new("ok", &format!("https://{}", "a".repeat(URL_MAX))).is_none());
    assert!(Button::new("o\u{202E}k", "https://example.com").is_none());
}

#[test]
fn an_activity_has_at_most_two_buttons() {
    let mut a = Activity::new(ActivityType::Playing);
    let b = Button::new("One", "https://example.com/1").unwrap();
    assert!(a.push_button(b.clone()));
    assert!(a.push_button(b.clone()));
    assert!(!a.push_button(b));
    assert_eq!(a.buttons().len(), 2);
}

#[test]
fn the_json_has_only_the_fields_with_values() {
    let a = Activity::new(ActivityType::Playing);
    assert_eq!(a.to_json(), serde_json::json!({ "type": 0 }));
    let mut a = Activity::new(ActivityType::Competing);
    a.details = Text::new("Explore · Race It");
    a.state = Text::new("Grand Golliat");
    a.start = Some(1_763_200_000);
    a.large_image = AssetKey::new("logo");
    a.large_text = Text::new("Project Avalanche");
    a.push_button(Button::new("Join the Discord", "https://discord.com/invite/XrFGQEfwdJ").unwrap());
    assert_eq!(
        a.to_json(),
        serde_json::json!({
            "type": 5,
            "details": "Explore · Race It",
            "state": "Grand Golliat",
            "timestamps": { "start": 1_763_200_000u64 },
            "assets": { "large_image": "logo", "large_text": "Project Avalanche" },
            "buttons": [{ "label": "Join the Discord", "url": "https://discord.com/invite/XrFGQEfwdJ" }],
        })
    );
}
