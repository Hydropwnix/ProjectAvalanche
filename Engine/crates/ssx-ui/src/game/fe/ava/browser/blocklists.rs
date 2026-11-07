use ssx_content::blocklist::Follow;
use ssx_net_join::blocklist::Refresh;

use crate::game::fe::FrontEnd;

pub fn start(fe: &mut FrontEnd) {
    let b = &mut fe.ava.browser;
    if b.fake.is_some() || b.blocklists.is_some() {
        return;
    }
    let Some(app_dir) = b.app_dir.clone() else { return };
    b.blocklists = Refresh::start(Follow::Player(app_dir), None);
}

pub fn poll(fe: &mut FrontEnd) {
    let Some(r) = fe.ava.browser.blocklists.as_ref() else { return };
    let Some(result) = r.take() else { return };
    fe.ava.browser.blocklists = None;
    match result {
        Ok(outcomes) => {
            for o in outcomes {
                fe.note(format!("browser: {}", o.line()));
            }
        }
        Err(e) => fe.note(format!("browser: the blocklists: {e}")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_blocked_mark_has_its_text() {
        let strings: std::collections::BTreeMap<String, String> =
            serde_json::from_str(include_str!("../../../../../../../assets/fe/strings/eng_us.json")).expect("strings");
        let text = strings.get("TXT_AVA_BROWSER_BLOCKED").expect("the key");
        assert_eq!(text.matches('%').count(), 1, "{text:?}");
        assert!(text.contains("%1s"), "{text:?}");
    }
}
