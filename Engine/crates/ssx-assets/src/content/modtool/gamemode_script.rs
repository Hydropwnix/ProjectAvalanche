use std::path::Path;

use anyhow::Result;

const SERVER_DEFS: &str = include_str!("../../../../ssx-script/defs/ssx-api1-server.d.luau");

pub(super) const MANIFEST_SCRIPTS: &str = r#"
# The gamemode's server script: in single player it runs in the game's local script host, on a server as one of its
# resources (never sent to a player).
[scripts]
server = ["server/*.luau"]

# What a player or a server's operator may change without touching the script (`get("name")` reads them).
[settings]
# A rider riding alone who falls this many seconds behind the best run at a gate has the round ended (0: never).
give_up_s = 8.0
"#;

pub(super) const SERVER_LUAU: &str = r#"-- The gamemode's server script: each rider's time at the course's gates against the best run kept in the package's
-- storage (one per map), told on the rider's HUD; a rider riding alone who falls more than `give_up_s` seconds behind
-- the best at a gate has the round ended; a faster finish is kept as the new best.

local GIVE_UP_S = tonumber(get("give_up_s")) or 0

type Run = { drop: string, gates: { number } }
type Best = { finish: number, gates: { number } }

-- the run of each rider in a running round
local runs: { [Element]: Run } = {}

local function secs(ms: number): string
    return string.format("%.2f", ms / 1000)
end

local function signed(s: number): string
    return (if s >= 0 then "+" else "") .. string.format("%.2f", s)
end

-- the best run on the map `drop` (nil: none yet)
local function bestOf(drop: string): Best?
    local all = storage.get("best")
    if type(all) ~= "table" then
        return nil
    end
    local b = all[drop]
    if type(b) ~= "table" or type(b.finish) ~= "number" or type(b.gates) ~= "table" then
        return nil
    end
    return b :: Best
end

local function keep(drop: string, best: Best)
    local all = storage.get("best")
    if type(all) ~= "table" then
        all = {}
    end
    (all :: { [string]: any })[drop] = best
    storage.set("best", all)
end

-- a round starts (`source`: its instance): each of its riders' run, the best told
addEventHandler("onRoundStart", root, function(round: number, drop: string)
    for _, rider in ipairs(getRiders(source)) do
        runs[rider] = { drop = drop, gates = {} }
    end
    local best = bestOf(drop)
    local text = if best then "Best " .. secs(best.finish) .. " s: beat it" else "No best yet: set one"
    showNotice(source, "Round " .. round, text, 3)
end)

-- a gate of the course (`checkpoint` nil: the course's own, not a script's checkpoint)
addEventHandler("onRiderCheckpoint", root, function(checkpoint: Element?, index: number, clock: number)
    local run = runs[source]
    local player = getRiderPlayer(source)
    if checkpoint ~= nil or run == nil or player == nil then
        return
    end
    run.gates[index + 1] = clock
    local best = bestOf(run.drop)
    local at = if best then best.gates[index + 1] else nil
    if at == nil then
        showNotice(player, "Gate " .. (index + 1), secs(clock) .. " s", 2)
        return
    end
    local gap = (clock - at) / 1000
    showNotice(player, "Gate " .. (index + 1), secs(clock) .. " s (" .. signed(gap) .. ")", 2)
    local instance = getPlayerInstance(player)
    if GIVE_UP_S > 0 and gap > GIVE_UP_S and instance and #getRiders(instance) == 1 then
        endRound(instance, string.format("%.1f s behind the best at gate %d", gap, index + 1))
    end
end)

-- a rider's finish: a faster one is the new best
addEventHandler("onRiderFinish", root, function(clock: number, points: number, failed: boolean)
    local run = runs[source]
    runs[source] = nil
    local player = getRiderPlayer(source)
    if failed or run == nil then
        return
    end
    local best = bestOf(run.drop)
    if best == nil or clock < best.finish then
        keep(run.drop, { finish = clock, gates = run.gates })
        if player then
            local was = if best then " (was " .. secs(best.finish) .. " s)" else ""
            showNotice(player, "New best", secs(clock) .. " s" .. was, 3)
        end
        print("a new best on " .. run.drop .. ": " .. secs(clock) .. " s")
    end
end)

addEventHandler("onRoundEnd", root, function(round: number)
    for _, rider in ipairs(getRiders(source)) do
        runs[rider] = nil
    end
end)
"#;

pub(super) fn write(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root.join("server"))?;
    std::fs::create_dir_all(root.join("types"))?;
    std::fs::write(root.join("server/main.luau"), SERVER_LUAU)?;
    std::fs::write(root.join("types/ssx-api1-server.d.luau"), SERVER_DEFS)?;
    Ok(())
}
