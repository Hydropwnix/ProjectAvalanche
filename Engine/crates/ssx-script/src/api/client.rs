use mlua::Lua;
use ssx_content::{ContentId, Kind};

use super::{ApiError, Args, Ctx, NameKind, Param, Ret, Sides, Spec, Ty, WORLD};
use crate::host::game::{GEAR_ALL, PROFILE_MAX, ProfileOrder, record, text, vec3_value};
use crate::value::NetValue;

const fn spec(name: &'static str, params: &'static [Param], returns: &'static [Ty], call: super::Imp) -> Spec {
    Spec { name, sides: Sides::Client, api: 1, params, rest: None, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

pub const KIND_NAMES: &[&str] = &[
    "texture",
    "model",
    "object",
    "material",
    "character",
    "gear",
    "board",
    "anim",
    "track",
    "drop",
    "event",
    "lighting",
    "sky",
    "postfx",
    "particles",
    "sound",
    "music",
    "movie",
    "ui",
    "font",
    "string",
    "tuning",
    "scoring",
    "script",
    "marker",
    "map",
    "gamemode",
    "trick",
];

pub const MAX_LIST_CONTENT: usize = 65_536;
const PRELOADED: [Kind; 3] = [Kind::Texture, Kind::Sound, Kind::Marker];
const SKY: f32 = 1.0e5;

const ID: Param = p("id", Ty::Name(NameKind::ContentId));

pub static SPECS: &[Spec] = &[
    spec("getLocalRider", &[], &[Ty::Opt(&Ty::Element)], local_rider),
    spec("getRiders", &[], &[Ty::List(&Ty::Element)], riders),
    spec("getRiderState", &[p("rider", Ty::Element)], &[Ty::Opt(&Ty::Record("RiderState"))], rider_state),
    spec("getInstance", &[], &[Ty::Opt(&Ty::Record("Instance"))], instance),
    spec("getStandings", &[], &[Ty::List(&Ty::Record("Standing"))], standings),
    spec("getContent", &[ID], &[Ty::Opt(&Ty::Record("ContentInfo"))], content),
    spec("listContent", &[p("kind", Ty::Enum(KIND_NAMES))], &[Ty::List(&Ty::Str { max: 256 })], list_content),
    spec("preloadContent", &[ID], &[Ty::Bool], preload),
    spec(
        "getGroundHeight",
        &[p("x", Ty::Number { min: -WORLD, max: WORLD }), p("z", Ty::Number { min: -WORLD, max: WORLD })],
        &[Ty::Opt(&Ty::Number { min: -WORLD, max: WORLD })],
        ground_height,
    ),
    spec("getProfile", &[], &[Ty::Opt(&Ty::Record("Profile"))], profile),
    spec("setCredits", &[p("credits", Ty::Int { min: 0, max: PROFILE_MAX })], &[Ty::Bool], set_credits),
    spec(
        "setRiderXp",
        &[p("rider", Ty::Str { max: 64 }), p("xp", Ty::Int { min: 0, max: PROFILE_MAX })],
        &[Ty::Bool],
        set_rider_xp,
    ),
    spec(
        "setRiderLevel",
        &[p("rider", Ty::Str { max: 64 }), p("level", Ty::Int { min: 1, max: 64 })],
        &[Ty::Bool],
        set_rider_level,
    ),
    spec("setInfiniteCredits", &[p("on", Ty::Bool)], &[Ty::Bool], set_infinite_credits),
    spec("captureMenuKeys", &[p("on", Ty::Bool)], &[Ty::Bool], capture_menu_keys),
    spec("setShopMinRarity", &[p("rarity", Ty::Int { min: 0, max: 4 })], &[Ty::Bool], set_shop_min_rarity),
    spec("setUnlimitedGear", &[p("mask", Ty::Int { min: 0, max: GEAR_ALL })], &[Ty::Bool], set_unlimited_gear),
    spec("setMaxLevel", &[p("level", Ty::Int { min: 1, max: 20 })], &[Ty::Bool], set_max_level),
    spec("getScreen", &[], &[Ty::Record("Screen")], screen),
    spec("getCursor", &[], &[Ty::Opt(&Ty::Record("Cursor"))], cursor),
    spec("isMultiplayer", &[], &[Ty::Bool], is_multiplayer),
    spec("isMainMenu", &[], &[Ty::Bool], is_main_menu),
    spec("drawOnTop", &[], &[Ty::Bool], draw_on_top),
    spec("getMainMenu", &[], &[Ty::Opt(&Ty::Record("MainMenu"))], main_menu),
    spec(
        "musicPlayer",
        &[p("action", Ty::Str { max: 16 }), p("id", Ty::Opt(&Ty::Str { max: 256 }))],
        &[Ty::Bool],
        music_player,
    ),
    spec("musicVolume", &[p("level", Ty::Number { min: 0.0, max: 1.0 })], &[Ty::Bool], music_volume),
    spec("openFolder", &[p("path", Ty::Str { max: 1024 })], &[Ty::Bool], open_folder),
];

fn local_rider(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let g = &w.game;
    let local = g.frame.riders.iter().find(|r| r.local.is_some()).and_then(|r| g.rider_element(r.key));
    Ok(vec![local.map_or(Ret::Nil, Ret::Element)])
}

fn riders(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let g = &w.game;
    Ok(vec![Ret::Elements(g.frame.riders.iter().filter_map(|r| g.rider_element(r.key)).collect())])
}

fn rider_state(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let el = a.element(0).ok_or("a rider expected")?;
    let host = ctx.host()?;
    let w = host.lock();
    if w.tree.get(el).is_none() {
        return Err(format!("element {el} does not exist (it was destroyed)").into());
    }
    let g = &w.game;
    let Some(r) = g.rider_key(el).and_then(|k| g.rider_view(k)) else { return Ok(vec![Ret::Nil]) };
    let l = r.local.as_ref();
    let num = |v: f64| Some(NetValue::Number(v));
    let v = record(vec![
        ("name", Some(text(&r.name))),
        ("position", Some(vec3_value(r.position))),
        ("velocity", l.map(|l| vec3_value(l.velocity))),
        ("speed", l.and_then(|l| num(f64::from(l.speed)))),
        ("air", l.map(|l| NetValue::Bool(l.air))),
        ("grind", l.map(|l| NetValue::Bool(l.grind))),
        ("trick", l.and_then(|l| l.trick.as_deref().map(text))),
        ("score", l.and_then(|l| num(l.score))),
        ("health", l.and_then(|l| num(f64::from(l.health.clamp(0.0, 1.0))))),
        ("finished", l.map(|l| NetValue::Bool(l.finished))),
        ("isLocal", Some(NetValue::Bool(l.is_some()))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn instance(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let Some(i) = &w.game.frame.instance else { return Ok(vec![Ret::Nil]) };
    let v = record(vec![
        ("mode", Some(text(&i.mode))),
        ("drop", Some(text(&i.drop))),
        ("state", Some(text(i.state))),
        ("clock", Some(NetValue::Number(i.clock_ms.max(0.0)))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn standings(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let g = &w.game;
    let rows = g.frame.standings.iter().enumerate().map(|(i, s)| {
        let row = record(vec![
            ("rider", s.rider.and_then(|k| g.rider_element(k)).map(NetValue::Element)),
            ("name", Some(text(&s.name))),
            ("place", Some(NetValue::Number(f64::from(s.place)))),
            ("value", Some(NetValue::Number(s.value))),
            ("finished", Some(NetValue::Bool(s.finished))),
        ]);
        (NetValue::Number((i + 1) as f64), row)
    });
    Ok(vec![Ret::Value(NetValue::Table(rows.collect()))])
}

fn content_id(a: &Args) -> Result<ContentId, ApiError> {
    a.str(0).ok_or("no id")?.parse::<ContentId>().map_err(|e| ApiError::Refused(e.to_string()))
}

fn content(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a)?;
    let host = ctx.host()?;
    let catalog = host.lock().game.catalog.clone();
    let Some(m) = catalog.ids.get(&id) else { return Ok(vec![Ret::Nil]) };
    let v = record(vec![
        ("id", Some(text(id.as_str()))),
        ("kind", Some(text(&id.kind().to_string()))),
        ("source", Some(text(&m.source))),
        ("layer", Some(text(m.layer))),
    ]);
    Ok(vec![Ret::Value(v)])
}

fn list_content(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let kind = a.str(0).and_then(Kind::parse).ok_or("no such kind")?;
    let host = ctx.host()?;
    let catalog = host.lock().game.catalog.clone();
    let n = catalog.kind(kind).count();
    if n > MAX_LIST_CONTENT {
        return Err(format!("{n} ids of kind {kind} (a list holds at most {MAX_LIST_CONTENT})").into());
    }
    Ok(vec![Ret::Strings(catalog.kind(kind).map(|id| id.as_str().to_string()).collect())])
}

fn preload(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let id = content_id(&a)?;
    if !PRELOADED.contains(&id.kind()) {
        return Ok(vec![Ret::Bool(false)]);
    }
    let host = ctx.host()?;
    let cap = host.limits.game.preload_frame;
    let mut w = host.lock();
    if !w.game.catalog.ids.contains_key(&id) {
        return Err(format!("{id}: no source provides it").into());
    }
    if w.game.orders.preload.len() >= cap {
        return Err(format!("{cap} preloads asked this frame (at most {cap})").into());
    }
    if !w.game.orders.preload.contains(&id) {
        w.game.orders.preload.push(id);
    }
    Ok(vec![Ret::Bool(true)])
}

fn ground_height(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (x, z) = (a.num(0).unwrap_or(0.0) as f32, a.num(1).unwrap_or(0.0) as f32);
    let host = ctx.host()?;
    let ground = host.lock().game.frame.ground.clone();
    let Some(ground) = ground else { return Ok(vec![Ret::Nil]) };
    let h = ground.height(x, SKY, z).filter(|h| h.is_finite() && f64::from(h.abs()) <= WORLD);
    Ok(vec![h.map_or(Ret::Nil, |h| Ret::Num(f64::from(h)))])
}

fn profile(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let Some(p) = &w.game.frame.profile else { return Ok(vec![Ret::Nil]) };
    let riders = p.riders.iter().enumerate().map(|(i, r)| {
        let row = record(vec![
            ("name", Some(text(&r.name))),
            ("displayName", Some(text(&r.display_name))),
            ("xp", Some(NetValue::Number(r.xp as f64))),
            ("level", Some(NetValue::Number(r.level as f64))),
        ]);
        (NetValue::Number((i + 1) as f64), row)
    });
    let v = record(vec![
        ("credits", Some(NetValue::Number(p.credits as f64))),
        ("infiniteCredits", Some(NetValue::Bool(p.infinite_credits))),
        ("unlimitedGear", Some(NetValue::Number(p.unlimited_gear as f64))),
        ("shopMinRarity", Some(NetValue::Number(p.shop_min_rarity as f64))),
        ("maxLevel", Some(NetValue::Number(p.max_level as f64))),
        ("riders", Some(NetValue::Table(riders.collect()))),
    ]);
    Ok(vec![Ret::Value(v)])
}

// A port addition, not the game's: a music player for mods. "play" (with a music id) starts that song now, in a
// ride or in the menus; "pause" holds the music where it is, "resume" carries on; "stop" stops it.
fn music_player(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    use crate::host::game::PlayerOrder;
    let action = a.str(0).ok_or("no action")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let order = match action {
        "play" => {
            let id = a.str(1).ok_or("play: no music id")?;
            let id = id.parse::<ContentId>().map_err(|e| ApiError::Refused(e.to_string()))?;
            if id.kind() != Kind::Music {
                return Err(format!("{id} is not a music id").into());
            }
            if !w.game.catalog.ids.contains_key(&id) {
                return Err(format!("{id}: no source provides it").into());
            }
            PlayerOrder::Play { package: ctx.package.clone(), id }
        }
        "pause" => PlayerOrder::Pause(true),
        "resume" => PlayerOrder::Pause(false),
        "stop" => PlayerOrder::Stop,
        "seek" => {
            let secs = a.str(1).and_then(|s| s.trim().parse::<f32>().ok()).ok_or("seek: no seconds")?;
            if !secs.is_finite() || secs < 0.0 {
                return Err("seek: seconds from 0".into());
            }
            PlayerOrder::Seek(secs)
        }
        other => return Err(format!("{other:?}: play, pause, resume, stop or seek").into()),
    };
    if w.game.orders.player.len() >= 8 {
        return Err("at most 8 musicPlayer calls a frame".into());
    }
    w.game.orders.player.push(order);
    Ok(vec![Ret::Bool(true)])
}

// A port addition, not the game's: the level of the licensed music (the game's songs and mods' songs), 0 to 1, on
// top of the game's own music volume. Back to 1 when the scripts stop.
fn music_volume(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    use crate::host::game::PlayerOrder;
    let level = a.num(0).ok_or("no level")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    w.game.orders.player.retain(|o| !matches!(o, PlayerOrder::Volume(_)));
    if w.game.orders.player.len() >= 8 {
        return Err("at most 8 musicPlayer calls a frame".into());
    }
    w.game.orders.player.push(PlayerOrder::Volume(level as f32));
    Ok(vec![Ret::Bool(true)])
}

// A port addition, not the game's: shows a folder on this PC in the system's file browser (Explorer), as a mod's
// "open my folder" button. Only an existing local folder (not a file, a link or a network path, so nothing is run
// and nothing is reached over the network), and at most once a second.
fn open_folder(_ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let raw = a.str(0).ok_or("no path")?;
    if raw.starts_with("\\\\") || raw.starts_with("//") || raw.contains('\0') {
        return Err("only a local folder".into());
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(format!("{raw}: not a full path").into());
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| ApiError::Refused(format!("{raw}: {e}")))?;
    if !meta.is_dir() {
        return Err(format!("{raw}: not a folder").into());
    }
    let real = std::fs::canonicalize(path).map_err(|e| ApiError::Refused(format!("{raw}: {e}")))?;
    let shown = real.to_string_lossy().to_string();
    // Windows gives the canonical path as \\?\C:\...; a network one as \\?\UNC\...
    let shown = match shown.strip_prefix("\\\\?\\") {
        Some(rest) if rest.starts_with("UNC\\") => return Err("only a local folder".into()),
        Some(rest) => rest.to_string(),
        None => shown,
    };
    let real = PathBuf::from(shown);
    if !real.is_dir() {
        return Err(format!("{raw}: not a folder").into());
    }
    {
        let mut last = LAST.lock().map_err(|_| "busy")?;
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return Ok(vec![Ret::Bool(false)]);
        }
        *last = Some(Instant::now());
    }
    let program = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(program).arg(&real).spawn() {
        Ok(_) => Ok(vec![Ret::Bool(true)]),
        Err(e) => Err(format!("{program}: {e}").into()),
    }
}

// A port addition, not the game's: the window's size and the part of the draw stage it shows, so a script's overlay
// can keep to what is on screen (the whole stage, at 1280 x 720, before the game has told).
fn screen(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let s = w.game.frame.screen.unwrap_or_default();
    let n = |v: f32| NetValue::Number(f64::from(v));
    let v = record(vec![
        ("width", Some(NetValue::Number(f64::from(s.width)))),
        ("height", Some(NetValue::Number(f64::from(s.height)))),
        ("left", Some(n(s.left))),
        ("top", Some(n(s.top))),
        ("right", Some(n(s.right))),
        ("bottom", Some(n(s.bottom))),
    ]);
    Ok(vec![Ret::Value(v)])
}

// A port addition, not the game's: where the mouse cursor is, in the units of the stage the scripts draw on (as
// `getScreen`); nil when it is outside the window.
fn cursor(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let Some((x, y)) = w.game.frame.cursor else { return Ok(vec![Ret::Nil]) };
    let n = |v: f32| Some(NetValue::Number(f64::from(v)));
    Ok(vec![Ret::Value(record(vec![("x", n(x)), ("y", n(y))]))])
}

// A port addition for mods (`isMultiplayer`): whether the player is in multiplayer (a network session, or the
// server browser before one), so a single-player tool can keep out of it.
fn is_multiplayer(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Bool(w.game.frame.multiplayer)])
}

// A port addition for mods (`isMainMenu`): whether the front end's main menu (the port's hub, or the game's own main
// menu) is the screen up, so a mod can show something there only.
fn is_main_menu(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    Ok(vec![Ret::Bool(w.game.frame.main_menu)])
}

// A port addition for mods (`getMainMenu`): on the port's hub, where its menu panel is (stage units), so a mod can
// place something by it; nil elsewhere.
fn main_menu(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let w = host.lock();
    let Some([x, y, wd, h]) = w.game.frame.main_menu_box else { return Ok(vec![Ret::Nil]) };
    let n = |v: f32| Some(NetValue::Number(f64::from(v)));
    Ok(vec![Ret::Value(record(vec![("x", n(x)), ("y", n(y)), ("width", n(wd)), ("height", n(h))]))])
}

// A port addition for mods (`drawOnTop`): this frame, the package's draws go over every other package's (one overlay
// sliding out over another sliding in). Only while onClientRender runs.
fn draw_on_top(ctx: &Ctx, _: &Lua, _: Args) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(list) = w.game.drawing.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    list.on_top.insert(ctx.package.clone());
    Ok(vec![Ret::Bool(true)])
}

// A port addition, not the game's: a single-player mod may set the profile's credits and its riders' XP. The frame
// carries no profile in a network session, so a server's client scripts are refused (`false`).
fn set_credits(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let credits = a.int(0).ok_or("no credits")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(p) = w.game.frame.profile.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    p.credits = credits;
    w.game.orders.profile.push(ProfileOrder::Credits { package: ctx.package.clone(), credits });
    Ok(vec![Ret::Bool(true)])
}

fn set_rider_xp(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no rider")?.to_string();
    let xp = a.int(1).ok_or("no xp")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(p) = w.game.frame.profile.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    let Some(r) = p.riders.iter_mut().find(|r| r.name.eq_ignore_ascii_case(&name)) else {
        return Err(format!("no rider named {name:?} in the profile").into());
    };
    r.xp = xp;
    let rider = r.key;
    w.game.orders.profile.push(ProfileOrder::RiderXp { package: ctx.package.clone(), rider, xp });
    Ok(vec![Ret::Bool(true)])
}

fn set_rider_level(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let name = a.str(0).ok_or("no rider")?.to_string();
    let level = a.int(1).ok_or("no level")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(p) = w.game.frame.profile.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    let max = p.max_level.max(1);
    let Some(r) = p.riders.iter_mut().find(|r| r.name.eq_ignore_ascii_case(&name)) else {
        return Err(format!("no rider named {name:?} in the profile").into());
    };
    r.level = level.min(max);
    let rider = r.key;
    w.game.orders.profile.push(ProfileOrder::RiderLevel { package: ctx.package.clone(), rider, level });
    Ok(vec![Ret::Bool(true)])
}

fn set_infinite_credits(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let on = a.bool(0).ok_or("no on")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(p) = w.game.frame.profile.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    p.infinite_credits = on;
    w.game.orders.profile.push(ProfileOrder::InfiniteCredits { package: ctx.package.clone(), on });
    Ok(vec![Ret::Bool(true)])
}

fn set_shop_min_rarity(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let rarity = a.int(0).ok_or("no rarity")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(p) = w.game.frame.profile.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    p.shop_min_rarity = rarity;
    w.game.orders.profile.push(ProfileOrder::ShopMinRarity { package: ctx.package.clone(), rarity });
    Ok(vec![Ret::Bool(true)])
}

// A port addition: the gear in `mask` never wears out or runs down on a drop, each piece its own bit (`GEAR_ALL`).
fn set_unlimited_gear(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let mask = a.int(0).ok_or("no mask")? & GEAR_ALL;
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(p) = w.game.frame.profile.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    p.unlimited_gear = mask;
    w.game.orders.profile.push(ProfileOrder::UnlimitedGear { package: ctx.package.clone(), mask });
    Ok(vec![Ret::Bool(true)])
}

// A port addition: riders may level past the game's last level, up to `level` (each new level free, its XP step the
// game's last). At most the game's count puts back the game's own.
fn set_max_level(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let level = a.int(0).ok_or("no level")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    let Some(p) = w.game.frame.profile.as_mut() else { return Ok(vec![Ret::Bool(false)]) };
    if level > p.max_level {
        p.max_level = level;
    }
    w.game.orders.profile.push(ProfileOrder::MaxLevel { package: ctx.package.clone(), level });
    Ok(vec![Ret::Bool(true)])
}

// A port addition: while a package holds them, the arrows, Enter and Backspace, and a gamepad's D-pad, left stick, A, B
// and right stick click reach the scripts only (a mod's menu drives with them). A package that stops lets go.
fn capture_menu_keys(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let on = a.bool(0).ok_or("no on")?;
    let host = ctx.host()?;
    let mut w = host.lock();
    if on {
        w.game.capture.insert(ctx.package.clone());
    } else {
        w.game.capture.remove(&ctx.package);
    }
    Ok(vec![Ret::Bool(true)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_names_are_the_contents() {
        let ours: Vec<String> = Kind::ALL.iter().map(ToString::to_string).collect();
        assert_eq!(ours, KIND_NAMES);
    }
}
