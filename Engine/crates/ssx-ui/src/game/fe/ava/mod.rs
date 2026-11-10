#[cfg(feature = "net")]
pub mod browser;
pub mod credits;
pub mod files;
pub mod hub;
pub mod keyboard;
pub mod maps;
#[cfg(feature = "net")]
pub mod name_prompt;
pub mod options;
pub mod port_settings;
#[cfg(feature = "net")]
pub mod resource_settings;
#[cfg(feature = "net")]
pub mod session;

#[derive(Default)]
pub struct Ava {
    pub keyboard: keyboard::Keyboard,
    pub hub: hub::Hub,
    pub options: options::Options,
    #[cfg(feature = "net")]
    pub name_prompt: name_prompt::NamePrompt,
    #[cfg(feature = "net")]
    pub confirmed_name: Option<String>,
    #[cfg(feature = "net")]
    pub profile_name_due: bool,
    pub maps: maps::Maps,
    #[cfg(feature = "net")]
    pub browser: browser::Browser,
    #[cfg(feature = "net")]
    pub filters: browser::filters::Page,
    #[cfg(feature = "net")]
    pub session: session::Session,
    pub credits: Vec<credits::Section>,
    pub third_party: credits::ThirdPartyList,
    pub glyphs: Option<std::collections::BTreeSet<char>>,
}

pub fn window_object(cx: &mut crate::avm1::Ctx, total: usize, first: usize) -> crate::avm1::value::Value {
    use crate::avm1::value::Value;
    let o = Value::Object(cx.new_object());
    let _ = cx.set_member(&o, "total", Value::Number(total as f64));
    let _ = cx.set_member(&o, "first", Value::Number(first as f64));
    o
}

pub fn profile_name(fe: &crate::game::fe::FrontEnd) -> String {
    #[cfg(feature = "net")]
    return name_prompt::join_name(fe);
    #[cfg(not(feature = "net"))]
    {
        let _ = fe;
        port_settings::DEFAULT_PLAYER_NAME.to_string()
    }
}

pub fn shown(text: &str, glyphs: Option<&std::collections::BTreeSet<char>>) -> String {
    let body: String = text
        .chars()
        .map(|c| match c {
            '<' | '>' | '&' => '?',
            c if c.is_control() || ssx_core::text::is_hidden(c) || crate::text::isolate::is_mark(c) => '?',
            c if glyphs.map_or(c.is_ascii_graphic() || c == ' ', |g| g.contains(&c) || crate::fallback::draws(c)) => c,
            _ => '?',
        })
        .collect();
    crate::text::isolate::wrap(&body)
}

pub fn bounded(text: &str, max: usize, glyphs: Option<&std::collections::BTreeSet<char>>) -> String {
    let cut: String = match text.chars().nth(max) {
        Some(_) => text.chars().take(max.saturating_sub(3)).chain("...".chars()).collect(),
        None => text.to_string(),
    };
    shown(&cut, glyphs)
}
