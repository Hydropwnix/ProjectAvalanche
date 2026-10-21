use super::*;

pub fn build(fe: &FrontEnd, cx: &mut Ctx, values: &UiValues, loc: &mut Localization, dp: &str) -> Option<Value> {
    match dp {
        "dp_globe_data" => Some(globe_data(fe, cx, values, loc)),
        "dp_drop_overlay_data" => Some(overlay(fe, cx, values)),
        _ => Some(Value::Object(cx.new_object())),
    }
}

pub(super) fn overlay(fe: &FrontEnd, cx: &mut Ctx, values: &UiValues) -> Value {
    let o = Value::Object(cx.new_object());
    let drop_level = matches!(fe.screen, 0x44 | 0xE | 0x11) && values.int(0xB3) == 3;
    let entries = fe.globe_scene.as_ref().map(|s| s.entries(STAGE, STAGE.0 / STAGE.1)).unwrap_or_default();
    let g = &fe.globe;
    let event = fe.career.event(g.provider_node).filter(|_| g.provider_is_drop && drop_level && !entries.is_empty());
    let Some(e) = event else {
        let _ = cx.set_member(&o, "isEnabled", Value::Bool(false));
        return o;
    };
    let locked = matches!(e.custom_status, 1 | 2) || !g.provider_selectable;
    let _ = cx.set_member(&o, "isEnabled", Value::Bool(true));
    let _ = cx.set_member(&o, "isCurrentDropLocked", Value::Bool(locked));
    let _ = cx.set_member(&o, "overlay", Value::str(&e.peak_short));
    let _ = cx.set_member(&o, "position", Value::str(&e.slot));
    o
}

pub fn launch_info(fe: &FrontEnd, drop: u32) -> Launch {
    let e = fe.career.event(drop);
    let track = e.and_then(|e| fe.career.drop(e.drop)).map(|d| d.track.clone()).unwrap_or_default();
    Launch {
        drop,
        event: e.map(|e| e.name.clone()).unwrap_or_default(),
        track,
        track_drop: 0,
        event_type: e.map_or(-1, |e| e.event_type),
        game_mode: e.map_or(-1, |e| match e.drop_type {
            DropType::Explore => 0,
            DropType::Campaign => 1,
            DropType::Type2 => 2,
            DropType::Custom => 3,
            DropType::None => -1,
        }),
        ..Default::default()
    }
}

pub(super) const CHARACTER_ICONS: [(u32, &str); 10] = [
    (0xF78F_F173, "assets/gamertagIcons/zoe_round.dds"),
    (0xB133_82C5, "assets/gamertagIcons/elise_round.dds"),
    (0xC163_B9A7, "assets/gamertagIcons/mac_round.dds"),
    (0xF51B_8616, "assets/gamertagIcons/alex_round.dds"),
    (0xA348_49CE, "assets/gamertagIcons/tane_round.dds"),
    (0x113F_EF1C, "assets/gamertagIcons/kaori_round.dds"),
    (0xBAD9_F705, "assets/gamertagIcons/psymon_round.dds"),
    (0xE96B_E6DD, "assets/gamertagIcons/moby_round.dds"),
    (0x0C50_6F77, "assets/gamertagIcons/ty_round.dds"),
    (0xCA56_8EDE, "assets/gamertagIcons/griff_round.dds"),
];

pub fn character_icon(fe: &FrontEnd, node: u32) -> String {
    const COMPLETE: &str = "assets/drop_selector_icons/campaign_complete.dds";
    const PLAYER: &str = "assets/gamertagIcons/player.dds";
    let map = |k: u32| CHARACTER_ICONS.iter().find(|(key, _)| *key == k).map(|(_, icon)| *icon);
    if fe.career.range(node).is_some() && fe.profile.range_modes.get(&node).is_some_and(|m| m[1] == 2) {
        return COMPLETE.into();
    }
    if let Some(icon) = map(node) {
        return icon.into();
    }
    let icon = if let Some(p) = fe.career.peak(node) {
        match p.campaign_type {
            1 => Some("assets/drop_selector_icons/peak_type_event.dds".to_string()),
            2 => Some("assets/drop_selector_icons/peak_type_dd.dds".to_string()),
            3 => Some("assets/drop_selector_icons/peak_type_backcountry.dds".to_string()),
            _ => map(p.range).map(str::to_string),
        }
    } else if let Some(e) = fe.career.event(node) {
        if let [rival] = e.rivals[..] {
            fe.career.riders.iter().find(|r| r.key == rival).map(|r| r.round_icon.clone())
        } else {
            let (peak, range) = fe.career.peak_and_range(e);
            map(peak).or_else(|| map(range)).map(str::to_string)
        }
    } else {
        None
    };
    icon.filter(|i| !i.is_empty()).unwrap_or_else(|| PLAYER.into())
}

pub(super) fn globe_data(fe: &FrontEnd, cx: &mut Ctx, values: &UiValues, loc: &mut Localization) -> Value {
    let entries = fe.globe_scene.as_ref().map(|s| s.entries(STAGE, STAGE.0 / STAGE.1)).unwrap_or_default();
    let complete = loc.text("TXT_GLOBE_COMPLETE");
    let new_label = loc.text("TXT_GLOBE_INDICATOR_NEW");
    let campaign = fe.game_mode == 1;
    let mut items = Vec::new();
    for e in entries {
        let name = match e.kind {
            2 => fe.career.peak(e.key).map(|p| loc.text(&p.title)).unwrap_or_default(),
            3 => loc.text(&e.name),
            _ => fe.career.range(e.key).map(|r| loc.text(&r.title)).unwrap_or_default(),
        };
        let state = match e.kind {
            3 => match fe.profile.drop_state(e.key) {
                DropState::Locked => 0,
                DropState::Unlocked => 1,
                DropState::Completed => 2,
            },
            _ => 1,
        };
        let (medal, percent) = match e.kind {
            2 => crate::game::fe::explore::fe_medal(fe, 2, e.key),
            3 => (-1, 0.0),
            _ => crate::game::fe::explore::fe_medal(fe, 1, e.key),
        };
        let o = Value::Object(cx.new_object());
        let num = |v: f64| Value::Number(v);
        let mut fields = vec![
            ("billboardid", num(f64::from(e.key as i32))),
            ("name", Value::str(&name)),
            ("completeLabel", Value::str(&complete)),
            ("newLabel", Value::str(&new_label)),
            ("medal", num(f64::from(medal))),
            ("medalPath", Value::str(crate::game::hud::results::medal_icon(medal))),
            ("percentage", num(f64::from(percent))),
            ("selected", Value::Bool(e.selected)),
            ("x", num(f64::from(e.x as i32))),
            ("y", num(f64::from(e.y as i32))),
            ("opacity", num(f64::from(e.opacity))),
            ("scale", num(f64::from(e.scale))),
            (
                "selectable",
                Value::Bool(e.selectable && (state == 1 || state == 2) && (e.kind != 1 || range_valid(fe, e.key))),
            ),
            ("winCondition", num(f64::from(e.win_condition))),
            ("isNew", Value::Bool(e.is_new)),
            ("isRecommended", Value::Bool(e.recommended && !campaign)),
            ("numFriendsGeoCaches", num(0.0)),
            ("isCampaign", Value::Bool(campaign)),
        ];
        if campaign {
            if let Some(ev) = fe.career.event(e.key) {
                let dd = ev.drop_type == DropType::Campaign && ev.event_type == 3;
                fields.push(("isCampaignDD", Value::Bool(dd)));
            }
            fields.push(("campaignChallengeText", Value::str("")));
            fields.push(("characterIcon", Value::str(&character_icon(fe, e.key))));
            if e.kind == 3 {
                fields.push(("isLocked", Value::Bool(state == 0 || (fe.progression.held == e.key && e.key != 0))));
                if state == 2 {
                    fields.push(("percentage", num(1.0)));
                }
            }
            if values.int(0xC0) == e.key as i32 {
                fields.push(("progressAnimType", num(f64::from(values.int(0xC1)))));
            }
        }
        fields.push(("isGlobalEvent", Value::Bool(fe.game_mode == 3)));
        fields.push(("restrictToSafezone", Value::Bool(e.kind == 2)));
        for (k, v) in fields {
            let _ = cx.set_member(&o, k, v);
        }
        items.push(o);
    }
    Value::Object(cx.new_array(items))
}
