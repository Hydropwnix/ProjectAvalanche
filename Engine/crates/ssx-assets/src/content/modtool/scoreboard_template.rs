use std::path::Path;

use anyhow::Result;

const CLIENT_DEFS: &str = include_str!("../../../../ssx-script/defs/ssx-api1-client.d.luau");
const SERVER_DEFS: &str = include_str!("../../../../ssx-script/defs/ssx-api1-server.d.luau");

fn manifest(name: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`. A server runs it as a
# resource: copy the folder into the server's resources/ and `start {name}` (or name it in server.toml).

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = "A scoreboard kept by the server and drawn by every player"
# The game's API level the scripts need (the definitions in types/ are this level's).
api = 1
type = "script"
priority = 0

# The server's scripts run on the server only (never sent to a player); the client's on every player's game.
[scripts]
server = ["server/*.luau"]
client = ["client/*.luau"]

# What the server's operator may change in server.toml without touching the scripts (`get("name")` reads them).
[settings]
# A round ends this many seconds after its winner crossed the finish (0: the round's own limit ends it).
after_winner = 30
# Names refused at the join, with a reason (any letters' case).
closed_names = ["intruder"]
"#
    )
}

const SERVER_LUAU: &str = r#"-- The scoreboard's server side. The API: types/ssx-api1-server.d.luau (luau-lsp), Docs/architecture/scripting.md §16.
-- Each player's row: the checkpoints passed, the finishes, the best time. The board goes to every player when it
-- changes, and to a player who asks (the remote event "scoreboard:ask", answered with "scoreboard:board" to that
-- player alone). The console command `scoreboard` prints the board, `scoreboard reset` clears it.

local AFTER_WINNER = tonumber(get("after_winner")) or 0
local CLOSED = {}
for _, n in ipairs(get("closed_names") or {}) do
    CLOSED[string.lower(n)] = true
end

local board = {}      -- player element -> row
local lastAsk = {}    -- player element -> os.clock() of its last ask
local leaders = {}    -- instance element -> { round, gate, clock }

-- a board started while a round runs (a restart after an edit) takes up each instance's round as it is, so the
-- winner's rule holds for that round too; `onRoundStart` replaces it with the next round
for _, instance in ipairs(getInstances()) do
    local info = getInstance(instance)
    if info then
        leaders[instance] = { round = info.round }
    end
end

-- remote: a client may send it (only this package's client scripts can: a remote event crosses between the two
-- sides of one package)
addEvent("scoreboard:ask", true)

local function rowOf(player)
    local r = board[player]
    if not r then
        r = { name = getPlayerName(player) or "?", checkpoints = 0, finishes = 0, best = 0 }
        board[player] = r
    end
    return r
end

local function rows()
    local list = {}
    for _, r in pairs(board) do
        table.insert(list, { name = r.name, checkpoints = r.checkpoints, finishes = r.finishes, best = r.best })
    end
    table.sort(list, function(a, b)
        if a.finishes ~= b.finishes then
            return a.finishes > b.finishes
        end
        if (a.best > 0) ~= (b.best > 0) then
            return a.best > 0
        end
        if a.best ~= b.best then
            return a.best < b.best
        end
        return a.name < b.name
    end)
    return list
end

local function sendBoard(to)
    triggerClientEvent(to, "scoreboard:board", root, rows())
end

local function seconds(ms: number): string
    return string.format("%d:%05.2f", ms // 60000, (ms % 60000) / 1000)
end

-- a join: refused for a closed name, else held a moment (as a check against a list elsewhere would hold it) and let
-- through; the server waits for the verdict at most 5 s
addEventHandler("onPlayerConnecting", root, function(name, keyId, join)
    if CLOSED[string.lower(name)] then
        print("join " .. join .. " refused: the name " .. name .. " is closed")
        refuseJoin(join, "the name " .. name .. " is closed on this server")
        return
    end
    deferJoin(join)
    updateJoin(join, "the scoreboard looks you up")
    setTimer(function()
        print("join " .. join .. " let through: " .. name)
        acceptJoin(join)
    end, 250, 1)
end)

addEventHandler("onPlayerJoin", root, function()
    rowOf(source)
    sendBoard(getPlayers())
end)

addEventHandler("onPlayerQuit", root, function(reason)
    board[source] = nil
    lastAsk[source] = nil
    sendBoard(getPlayers())
end)

-- the round trip's server half: `client` is the player whose script sent it; one answer a second at most
addEventHandler("scoreboard:ask", root, function()
    if not client then
        return
    end
    local now = os.clock()
    if lastAsk[client] and now - lastAsk[client] < 1 then
        return
    end
    lastAsk[client] = now
    print((getPlayerName(client) or "?") .. " asked for the board")
    sendBoard(client)
end)

addEventHandler("onRoundStart", root, function(round, drop)
    local old = leaders[source]
    if old and isElement(old.gate) then
        destroyElement(old.gate)
    end
    leaders[source] = { round = round }
end)

-- `checkpoint` is nil for the course's own splits, the gate for the round leader's
addEventHandler("onRiderCheckpoint", root, function(checkpoint, index, clock)
    local player = getRiderPlayer(source)
    if not player then
        return
    end
    local instance = getPlayerInstance(player)
    local lead = instance and leaders[instance]
    if checkpoint == nil then
        rowOf(player).checkpoints += 1
        sendBoard(getPlayers())
        -- the first rider of the round past the first split leaves a gate of its instance there
        local state = getRiderState(source)
        if index == 1 and lead and not lead.gate and state and state.position then
            lead.gate = createCheckpoint("ava:marker/gate", state.position, 1, { instance = instance, size = 12 })
            lead.clock, lead.player = clock, player
            print(rowOf(player).name .. " leads round " .. lead.round .. ": a gate where it passed the first split")
        end
    elseif lead and checkpoint == lead.gate and player ~= lead.player then
        local gap = math.max(0, clock - (lead.clock or 0))
        print(rowOf(player).name .. " passed the leader's gate +" .. seconds(gap))
        showNotice(player, "LEADER'S GATE", "+" .. seconds(gap) .. " behind the leader", 2)
    end
end)

addEventHandler("onRiderFinish", root, function(clock, points, failed)
    local player = getRiderPlayer(source)
    if not player or failed then
        return
    end
    local r = rowOf(player)
    r.finishes += 1
    if r.best == 0 or clock < r.best then
        r.best = clock
    end
    print(r.name .. " finished in " .. seconds(clock))
    showNotice(player, "FINISH", seconds(clock) .. (clock == r.best and "  your best" or ""), 3)
    sendBoard(getPlayers())
    -- the round's winner: the others have AFTER_WINNER seconds left
    local instance = getPlayerInstance(player)
    local lead = instance and leaders[instance]
    if AFTER_WINNER > 0 and lead and not lead.winner then
        lead.winner = r.name
        local round = lead.round
        -- an instance as the recipient: its players
        showNotice(instance, "WINNER", r.name .. ": " .. AFTER_WINNER .. " s left", 3)
        setTimer(function()
            local info = getInstance(instance)
            -- the same round still running (a round that ended, or the next one, is left alone)
            if info and info.round == round and info.state == "running" then
                print("round " .. round .. " ended " .. AFTER_WINNER .. " s after its winner")
                endRound(instance, AFTER_WINNER .. " s after the winner")
            end
        end, AFTER_WINNER * 1000, 1)
    end
end)

-- the server's console: `scoreboard` prints the board, `scoreboard reset` clears it (a player's command too)
addCommandHandler("scoreboard", function(player, command, what)
    if what == "reset" then
        for p in pairs(board) do
            board[p] = { name = board[p].name, checkpoints = 0, finishes = 0, best = 0 }
        end
        sendBoard(getPlayers())
        print("the scoreboard was reset")
        return
    end
    for i, r in ipairs(rows()) do
        local best = r.best > 0 and seconds(r.best) or "-"
        print(string.format("%2d. %-24s %3d checkpoints %2d finishes  best %s", i, r.name, r.checkpoints, r.finishes, best))
    end
end)
"#;

const CLIENT_LUAU: &str = r#"-- The scoreboard's client side. The API: types/ssx-api1-client.d.luau (luau-lsp), Docs/architecture/scripting.md.
-- It asks the server for the board when it starts (the remote event "scoreboard:ask"), takes every board the server
-- sends ("scoreboard:board"), and draws it at the stage's left while F3 is held.

local rows = {}
local shown = false

-- remote: the server may send it to this package's client scripts
addEvent("scoreboard:board", true)

addEventHandler("scoreboard:board", root, function(list)
    if type(list) == "table" then
        if #rows == 0 then
            print("the first board: " .. #list .. " players")
        end
        rows = list
    end
end)

addEventHandler("onClientResourceStart", resourceRoot, function()
    -- the round trip's client half: the server answers this client alone
    triggerServerEvent("scoreboard:ask", localPlayer)
end)

bindKey("f3", "both", function(key, state)
    shown = state == "down"
end)

local function seconds(ms: number): string
    return string.format("%d:%05.2f", ms // 60000, (ms % 60000) / 1000)
end

addEventHandler("onClientRender", root, function()
    -- one line always (the whole board while F3 is held), on the HUD's 1280 x 720 stage below the race HUD's
    -- top-left block (its clock, speed and distance end near y 150)
    if not shown then
        draw.text(string.format("SCOREBOARD  %d players  (F3)", #rows), 40, 164, tocolor(255, 200, 80), { size = 16, shadow = true })
        return
    end
    local height = 40 + 24 * math.min(#rows, 16)
    draw.rect(40, 176, 460, height, tocolor(0, 0, 0, 170))
    draw.text("SCOREBOARD", 56, 182, tocolor(255, 200, 80), { size = 22 })
    for i, r in ipairs(rows) do
        if i > 16 then
            break
        end
        local best = (r.best or 0) > 0 and seconds(r.best) or "-"
        local line = string.format("%d. %s   %d cp   %d fin   %s", i, tostring(r.name), r.checkpoints or 0, r.finishes or 0, best)
        draw.text(line, 56, 186 + 24 * i, tocolor(255, 255, 255), { size = 18 })
    end
end)
"#;

fn readme(name: &str) -> String {
    format!(
        r#"{name}
{under}

A server resource for Project Avalanche: a scoreboard kept by the server and drawn by every player (hold F3).

- server/main.luau (runs on the server only): each player's checkpoints, finishes and best time; the board sent to
  every player when it changes and to a player who asks (`scoreboard:ask`, a remote event: the round trip); the
  console command `scoreboard` (and `scoreboard reset`); the round's leader leaves a gate of its instance where it
  passed the first split, each rider who passes it is told the gap; a round ends `after_winner` seconds after its
  winner; a join is held a moment and let through, or refused for a name in `closed_names`.
- client/main.luau (runs on every player's game): asks for the board when it starts, keeps each board the server
  sends, draws it while F3 is held.
- types/: the API's type definitions of both sides for your editor.

Check it with `ssx-assets mod check <folder>`: it compiles both sides' scripts and checks that each remote event one
side sends is declared remote on the other (`addEvent(name, true)`). Run it on a server: copy the folder into the
server's `resources/` and type `start {name}` at its console (or name it in `server.toml`'s resources); after a change,
`refresh` and `restart {name}`. The server's console command `scripts` shows the scripts' monitor.

A remote event is a door: a player can send it with any arguments, as often as the server's limits let it. The server
side checks what it is sent (here: nothing but a rate) and never trusts a client's word for a result.
"#,
        under = "=".repeat(name.len())
    )
}

pub fn scoreboard(root: &Path, name: &str) -> Result<()> {
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name))?;
    for dir in ["server", "client", "types"] {
        std::fs::create_dir(root.join(dir))?;
    }
    std::fs::write(root.join("server/main.luau"), SERVER_LUAU)?;
    std::fs::write(root.join("client/main.luau"), CLIENT_LUAU)?;
    std::fs::write(root.join("types/ssx-api1-client.d.luau"), CLIENT_DEFS)?;
    std::fs::write(root.join("types/ssx-api1-server.d.luau"), SERVER_DEFS)?;
    std::fs::write(root.join("README.md"), readme(name))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{Template, new_package};

    #[test]
    fn the_scoreboard_template_passes_the_package_check() {
        let out = std::env::temp_dir().join(format!("ssx-scoreboard-template-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let root = new_package("my-board", Template::Scoreboard, &out, std::path::Path::new("")).unwrap();
        let c = ssx_content::check::check_package(&root, &ssx_content::texture::Limits::default());
        let errors: Vec<String> = c.findings.iter().map(ToString::to_string).filter(|f| f.contains("error")).collect();
        assert!(errors.is_empty(), "{errors:#?}");
        let m = c.manifest.as_ref().unwrap();
        assert_eq!((m.scripts.server.len(), m.scripts.client.len()), (1, 1));
        assert!(m.settings.contains_key("after_winner") && m.settings.contains_key("closed_names"));
        let _ = std::fs::remove_dir_all(&out);
    }
}
