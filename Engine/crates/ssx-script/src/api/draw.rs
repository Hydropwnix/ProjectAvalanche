use mlua::Lua;
use ssx_content::{ContentId, Kind};

use super::{ApiError, Arg, Args, COLOUR, Ctx, NameKind, Param, Ret, Sides, Spec, Ty};
use crate::host::game::{Align, Draw};

const fn spec(name: &'static str, params: &'static [Param], returns: &'static [Ty], call: super::Imp) -> Spec {
    Spec { name, sides: Sides::Client, api: 1, params, rest: None, returns, any_results: false, call }
}

const fn p(name: &'static str, ty: Ty) -> Param {
    Param { name, ty }
}

const XY: Ty = Ty::Number { min: -4096.0, max: 4096.0 };
const WH: Ty = Ty::Number { min: 0.0, max: 4096.0 };
const BYTE: Ty = Ty::Int { min: 0, max: 255 };
const B: &[Ty] = &[Ty::Bool];

pub static SPECS: &[Spec] = &[
    spec(
        "draw.text",
        &[
            p("text", Ty::Str { max: 256 }),
            p("x", XY),
            p("y", XY),
            p("color", Ty::Opt(&COLOUR)),
            p("options", Ty::Opt(&Ty::Record("TextOptions"))),
        ],
        B,
        text,
    ),
    spec("draw.rect", &[p("x", XY), p("y", XY), p("w", WH), p("h", WH), p("color", COLOUR)], B, rect),
    spec(
        "draw.image",
        &[
            p("x", XY),
            p("y", XY),
            p("w", WH),
            p("h", WH),
            p("texture", Ty::Name(NameKind::ContentId)),
            p("color", Ty::Opt(&COLOUR)),
        ],
        B,
        image,
    ),
    spec("tocolor", &[p("r", BYTE), p("g", BYTE), p("b", BYTE), p("a", Ty::Opt(&BYTE))], &[COLOUR], tocolor),
];

fn f(a: &Args, i: usize) -> f32 {
    a.num(i).unwrap_or(0.0) as f32
}

fn colour(a: &Args, i: usize, default: u32) -> Result<u32, ApiError> {
    match a.int(i) {
        Some(c) => u32::try_from(c).map_err(|_| "a colour beyond 32 bits".into()),
        None => Ok(default),
    }
}

fn push(ctx: &Ctx, d: Draw, glyphs: usize, pixels: f64) -> Result<Vec<Ret>, ApiError> {
    let host = ctx.host()?;
    let caps = host.limits.game;
    let mut w = host.lock();
    let Some(list) = w.game.drawing.as_mut() else {
        return Err("draws are taken only while onClientRender runs".into());
    };
    let mine = list.per_package.get(&ctx.package).copied().unwrap_or(0);
    if list.items.len() >= caps.draw_calls || mine >= caps.draw_calls_package {
        return Err(format!(
            "the frame holds {} draw calls, {mine} of this package (at most {} and {})",
            list.items.len(),
            caps.draw_calls,
            caps.draw_calls_package
        )
        .into());
    }
    if list.glyphs + glyphs > caps.draw_glyphs {
        return Err(format!(
            "the frame's text would be {} glyphs (at most {})",
            list.glyphs + glyphs,
            caps.draw_glyphs
        )
        .into());
    }
    let max_pixels = caps.draw_pixels as f64;
    if list.pixels + pixels > max_pixels {
        return Err(format!(
            "the frame's rects and images would cover {:.0} pixels (at most {:.0})",
            list.pixels + pixels,
            max_pixels
        )
        .into());
    }
    list.glyphs += glyphs;
    list.pixels += pixels;
    *list.per_package.entry(ctx.package.clone()).or_default() += 1;
    list.items.push((ctx.package.clone(), d));
    Ok(vec![Ret::Bool(true)])
}

fn text(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let s = a.str(0).ok_or("no text")?.to_string();
    let glyphs = s.chars().count();
    let size = match a.field(4, "size") {
        Some(Arg::Num(n)) => *n as f32,
        _ => 16.0,
    };
    let align = match a.field(4, "align") {
        Some(Arg::Str(s)) if s == "center" => Align::Center,
        Some(Arg::Str(s)) if s == "right" => Align::Right,
        _ => Align::Left,
    };
    let shadow = matches!(a.field(4, "shadow"), Some(Arg::Bool(true)));
    let d = Draw::Text { text: s, x: f(&a, 1), y: f(&a, 2), colour: colour(&a, 3, 0xFFFF_FFFF)?, size, align, shadow };
    push(ctx, d, glyphs, 0.0)
}

fn rect(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let (w, h) = (f(&a, 2), f(&a, 3));
    let d = Draw::Rect { x: f(&a, 0), y: f(&a, 1), w, h, colour: colour(&a, 4, 0xFFFF_FFFF)? };
    push(ctx, d, 0, f64::from(w) * f64::from(h))
}

fn image(ctx: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let texture = a.str(4).ok_or("no texture")?.parse::<ContentId>().map_err(|e| ApiError::Refused(e.to_string()))?;
    if texture.kind() != Kind::Texture {
        return Err(format!("{texture} is not a texture id").into());
    }
    if !ctx.host()?.lock().game.catalog.ids.contains_key(&texture) {
        return Err(format!("{texture}: no source provides it").into());
    }
    let (w, h) = (f(&a, 2), f(&a, 3));
    let d = Draw::Image { x: f(&a, 0), y: f(&a, 1), w, h, texture, colour: colour(&a, 5, 0xFFFF_FFFF)? };
    push(ctx, d, 0, f64::from(w) * f64::from(h))
}

fn tocolor(_: &Ctx, _: &Lua, a: Args) -> Result<Vec<Ret>, ApiError> {
    let byte = |i: usize, default: i64| a.int(i).unwrap_or(default) as u32 & 0xFF;
    let c = byte(3, 255) << 24 | byte(0, 0) << 16 | byte(1, 0) << 8 | byte(2, 0);
    Ok(vec![Ret::Num(f64::from(c))])
}
