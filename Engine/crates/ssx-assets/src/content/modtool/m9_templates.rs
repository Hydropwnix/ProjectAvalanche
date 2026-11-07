use std::path::Path;

use anyhow::Result;

pub const EVENT: &str = "ssx:event/exp_antarctica_craddock_drop_7_survive";

fn manifest(name: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, §21).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = "Faster tucks, longer combos and nearer medal distances on Mount Craddock's Headshot"
api = 1
type = "mod"
priority = 0

# A patch names only the fields it changes, by their decoded names; `ssx-assets content export <id>` writes a whole
# record to start from (for your own use). Tuning, scoring, events and drops are changed, never added.
[patch]
"ssx:tuning/characterpropertiesasset/cp_max" = "data/tuck.toml"
"ssx:scoring/scoringsceneopasset/scoringsceneop" = "data/combo.toml"
"{EVENT}" = "data/medals.toml"
"#
    )
}

const TUCK: &str = r#"# A patch of ssx:tuning/characterpropertiesasset/cp_max: the riders' properties at the top of their stats (each
# rider's value lies between CP_Min's and CP_Max's by the rider's stat). Simulation: a ride with it is not the base
# game's, and a server's sim content id tells.
# The disc's value is 1.15 (`ssx-assets content export ssx:tuning/characterpropertiesasset/cp_max` shows every field
# with its type and range).
TuckedVelocityMultiplier = 1.25
"#;

const COMBO: &str = r#"# A patch of ssx:scoring/scoringsceneopasset/scoringsceneop: the scoring's rules. TrickComboLinkTime is how long (in
# ticks, 60 a second) a landed trick waits for the next one to join the combo; the disc's is 240.
TrickComboLinkTime = 300
"#;

const MEDALS: &str = r#"# A patch of an event's rivals: `ghostdb.csv`'s rows of this ui_drop, by their row (0 first). `score` is the
# rival's value: the time in milliseconds on a race, the points on a trick event, the distance descended in
# millimetres on a survive event. Mount Craddock's "Headshot" is a survive event: the run's descent is ranked against
# the three scores (beat one for bronze, two for silver, all three for gold), and the drop screen's medal targets show
# them. The disc's are 3620851 (row 0, silver), 2068391 (row 1, bronze) and 5382051 (row 2, gold); these put the
# medals at 1 km, 250 m and 2.5 km, so a run that ends early still earns bronze. Only the rows and columns named here
# change; `ssx-assets content export ssx:event/exp_antarctica_craddock_drop_7_survive` shows the event whole.
[rivals.0]
score = 1000000

[rivals.1]
score = 250000

[rivals.2]
score = 2500000
"#;

const README: &str = r#"# A data package

Three patches of the game's data records (Docs/architecture/content.md §21):

- `data/tuck.toml` patches `ssx:tuning/characterpropertiesasset/cp_max`: a faster tuck for the riders at the top of
  their stats.
- `data/combo.toml` patches `ssx:scoring/scoringsceneopasset/scoringsceneop`: a longer window to link tricks.
- `data/medals.toml` patches the rivals of Mount Craddock's "Headshot" survive drop
  (`exp_antarctica_craddock_drop_7_survive`, Antarctica): their scores are the drop's medal distances (gold 5 382 m,
  silver 3 621 m, bronze 2 068 m on the disc; 2.5 km, 1 km and 250 m with the patch). The drop screen's medal
  targets show the new distances, and a run is ranked against them: one that descends 300 m earns bronze instead of
  no medal, one that descends 3 km gold instead of bronze.

`ssx-assets content ids --kind tuning` (also `scoring`, `event`, `drop`) lists the records; `ssx-assets content
export <id>` writes one whole, each field under a comment with its type, its range and whether the simulation reads
it. `ssx-assets mod check <folder>` checks every field against the record's schema and names the file, line and
column of each problem; the game refuses a record that does not fit, says why, and uses the disc's.

All three change the simulation: in single player they apply, and a result ridden with them carries the session's
sim content id (`ssx-assets content sim-id antarc0805 exp_antarctica_craddock_drop_7_survive` prints it and its
inputs); a server whose policy is `client_mods = "presentation"` switches them off for its sessions.

With `--mod-dev <folder>` the game re-reads a changed record at the next ride start (a ride reads its rules once,
when it starts).
"#;

pub(super) fn data(root: &Path, name: &str) -> Result<()> {
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name))?;
    std::fs::create_dir_all(root.join("data"))?;
    std::fs::write(root.join("data/tuck.toml"), TUCK)?;
    std::fs::write(root.join("data/combo.toml"), COMBO)?;
    std::fs::write(root.join("data/medals.toml"), MEDALS)?;
    std::fs::write(root.join("README.md"), README)?;
    Ok(())
}
