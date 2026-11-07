pub const NAME: &str = "gully_race";
pub const GAMEMODE_PATH: &str = "gamemodes/gully_race.toml";

pub const TARGETS_S: [f64; 3] = [13.0, 15.0, 20.0];
pub const TIME_LIMIT_S: f64 = 90.0;

pub fn gamemode_toml(map: &str) -> String {
    let [gold, silver, bronze] = TARGETS_S;
    format!(
        "# A gamemode: the rules a map is ridden by. The game and a server run them without a script; the package's\n\
         # server scripts (resource.toml [scripts] server) add to them in the local script host.\n\
         [gamemode]\n\
         title = \"Gully Race\"\n\
         # one of trick_score, trick_bots, trick_ghosts, survive_to_bottom, survive_to_death, race_clock, race_ghosts,\n\
         # race_bots, freeride (the disc's nine event types)\n\
         event = \"race_ghosts\"\n\
         # what the time limit's end does: fail (the run ends failed) or result (it ends with its value; not for races)\n\
         time_up = \"fail\"\n\
         \n\
         # each map the gamemode takes, with its targets (none or three: gold, silver, bronze, best first; a race's in\n\
         # seconds, a trick's in points, a survive's in metres) and its time limit in seconds\n\
         [[gamemode.map]]\n\
         id = \"{map}\"\n\
         targets = [{gold:.1}, {silver:.1}, {bronze:.1}]\n\
         time_limit_s = {TIME_LIMIT_S:.1}\n"
    )
}
