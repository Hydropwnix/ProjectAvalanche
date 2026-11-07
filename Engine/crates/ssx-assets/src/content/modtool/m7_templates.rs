use std::path::Path;

use anyhow::Result;

const CLIENT_DEFS: &str = include_str!("../../../../ssx-script/defs/ssx-api1-client.d.luau");

fn manifest(name: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = "A gate course laid ahead of the rider, timed on the HUD"
# The game's API level the scripts need (the definitions in types/ are this level's).
api = 1
type = "script"
priority = 0

# The package's own marker: a gate, its look and the sound it plays when passed (markers/gate.toml).
[add]
"marker/gate" = "markers/gate.toml"

# Client scripts run in single player, and on a server only where its client_scripts policy allows them.
[scripts]
client = ["client/*.luau"]

# Modules the client scripts `require` (a client receives only what [scripts] and [files] include name).
[files]
include = ["lib/*.luau"]
"#
    )
}

const GATE_TOML: &str = r##"# A gate of this package (every key: Docs/architecture/content.md §19.2): the port's built-in gate shape, 20 m wide.
shape = "gate"
size = 20.0
color = "#FF6030E0"
glow = 1.5
facing = "world"
draw_distance = 800.0
# Played when the rider passes it (the base game's sound, named by id, never copied).
sound = "ssx:sound/tuna/hudsfx_tuna/geotag_collected"

[animation]
pulse = 0.05
pulse_period = 1.0
"##;

fn main_luau(name: &str) -> String {
    format!(
        r#"-- A gate course laid ahead of the rider as it rides (lib/course.luau), timed on the HUD; each scored trick with its
-- points, a big trick and a big landing in the HUD's message. The API: types/ssx-api1-client.d.luau (luau-lsp),
-- Docs/architecture/scripting.md. draw.text draws in the game's font (a character it lacks shows as ?).
local course = require("lib/course")

local GATES = 10

local state = course.new("{name}:marker/gate", GATES)
local trick, trickAt = "", -10.0

addEventHandler("onClientRender", root, function()
    local me = getLocalRider()
    if not me then
        return
    end
    local s = getRiderState(me)
    if not s then
        return
    end
    course.update(state, s, os.clock())
    if state.started then
        local t = state.time
        -- below the HUD's leaderboard, at the stage's right
        draw.rect(944, 268, 296, 74, tocolor(0, 0, 0, 150))
        draw.text(string.format("%d:%05.2f", t // 60, t % 60), 960, 272, tocolor(255, 255, 255), {{ size = 34 }})
        local line = string.format("GATES %d / %d   MISSED %d", state.passed, GATES, state.missed)
        draw.text(line, 960, 312, tocolor(255, 200, 80), {{ size = 18 }})
    end
    if os.clock() - trickAt < 2 then
        draw.text(trick, 640, 380, tocolor(120, 220, 255), {{ size = 28, align = "center", shadow = true }})
    end
end)

addEventHandler("onClientMarkerHit", root, function(rider, forward)
    if rider == getLocalRider() and forward then
        course.passed(state, source)
    end
end)

addEventHandler("onClientTrick", root, function(name, points)
    trick, trickAt = string.format("%s  +%d", name, points), os.clock()
    if points >= 5000 then
        -- the HUD takes one message a second from a package: one refused now is no reason to stop
        pcall(showHudMessage, "BIG TRICK", name, 2)
    end
end)

addEventHandler("onClientRiderLand", root, function(airTime, bail)
    if airTime > 1.5 and not bail then
        pcall(showHudMessage, "BIG AIR", string.format("%.1f seconds in the air", airTime), 2)
    end
end)
"#
    )
}

const COURSE_LUAU: &str = r#"-- The course: the next gate 50 m ahead of the rider along its line, on the ground, placed while the rider rides
-- (not in the air); a gate passed, or left more than 25 m behind, gives way to the next, until the course is done.
local M = {}

local AHEAD = 50
local BEHIND = 25

function M.new(marker: string, gates: number)
    return { marker = marker, gates = gates, placed = 0, passed = 0, missed = 0, current = nil, at = nil,
             dir = nil, started = false, done = false, start = 0, time = 0 }
end

local function place(c, s)
    local v = s.velocity
    if not v then
        return
    end
    local len = math.sqrt(v.x * v.x + v.z * v.z)
    if len < 1 then
        return
    end
    local dx, dz = v.x / len, v.z / len
    local x, z = s.position.x + dx * AHEAD, s.position.z + dz * AHEAD
    local y = getGroundHeight(x, z) or s.position.y
    c.current = createMarker(c.marker, { x = x, y = y, z = z }, { heading = math.deg(math.atan2(dx, dz)) })
    c.at, c.dir = { x = x, z = z }, { x = dx, z = dz }
    c.placed += 1
end

local function advance(c, s)
    if c.current then
        destroyElement(c.current)
        c.current = nil
    end
    if c.placed >= c.gates then
        c.done = true
    elseif not s.air then
        place(c, s)
    end
end

function M.update(c, s, now: number)
    if c.done then
        return
    end
    -- a gate waits for the rider to be back on the snow
    if c.started and not c.current and c.placed < c.gates and not s.air then
        place(c, s)
    end
    if not c.started then
        if (s.speed or 0) > 8 then
            c.started, c.start = true, now
            place(c, s)
        end
        return
    end
    c.time = now - c.start
    if c.current and c.at then
        local past = (s.position.x - c.at.x) * c.dir.x + (s.position.z - c.at.z) * c.dir.z
        if past > BEHIND then
            c.missed += 1
            advance(c, s)
        end
    end
end

function M.passed(c, gate)
    if gate == c.current then
        c.passed += 1
        advance(c, getRiderState(getLocalRider()))
    end
end

return M
"#;

fn readme(name: &str) -> String {
    format!(
        r#"{name}
{under}

A client script for Project Avalanche: a gate course laid ahead of the rider as it rides, timed on the HUD.

- client/main.luau: the events it handles (onClientRender draws the timer, onClientMarkerHit counts a gate,
  onClientTrick and onClientRiderLand show a trick and a big landing).
- lib/course.luau: the course (a module the main script requires).
- markers/gate.toml: the package's own gate (the port's gate shape, its colour and the sound it plays when passed).
- types/ssx-api1-client.d.luau: the API's type definitions for your editor.

Type checking is your editor's: the game compiles the scripts (syntax) but does not type-check them (Luau's type
checker is not part of the game). With luau-lsp, check a script against the definitions:

    luau-lsp analyze --platform=standard --definitions=@ssx=types/ssx-api1-client.d.luau client/main.luau

Check the package with `ssx-assets mod check <folder>` (it compiles every script and resolves every `require`), pack
it with `ssx-assets mod pack <folder>`, install it from the launcher's Mods page. While you write it,
`avalanche-game --mod-dev <folder>` restarts the scripts when a file changes; F9 shows the script monitor.

Client scripts run in single player, and on a server only where its client_scripts policy allows them.
"#,
        under = "=".repeat(name.len())
    )
}

pub fn script(root: &Path, name: &str) -> Result<()> {
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name))?;
    for dir in ["client", "lib", "markers", "types"] {
        std::fs::create_dir(root.join(dir))?;
    }
    std::fs::write(root.join("client/main.luau"), main_luau(name))?;
    std::fs::write(root.join("lib/course.luau"), COURSE_LUAU)?;
    std::fs::write(root.join("markers/gate.toml"), GATE_TOML)?;
    std::fs::write(root.join("types/ssx-api1-client.d.luau"), CLIENT_DEFS)?;
    std::fs::write(root.join("README.md"), readme(name))?;
    Ok(())
}
