pub const TICKS_PER_SECOND: f32 = 60.0;
pub const SIM_HZ: f32 = 30.0;
pub const DT_TICKS: f32 = TICKS_PER_SECOND / SIM_HZ;
pub const KMH_PER_M_PER_TICK: f32 = 216.0;
pub const MS2_PER_M_PER_TICK2: f32 = 3600.0;

pub const PORT_SUMMARY: &str = "from the game: the level start (SF_default_level_start: ready loop placed by the \
TrajectoryOffsetController, GO on A / right stick up / the get-going gesture, the go clip's root motion, the \
hand-over to SF_Ride at its tick 40), trajectory point held at the riding height (landing absorption, riding-height \
spring, board-sphere floor), 13-query ground probe + ridable selection, ground/air states (1.4/1.3 m), collision \
correction (24-ray fan, push-out, bounce, soft deflection), steering PIDs + slip state + dig friction, jump timers + \
launch + obstacle fan + compression, air jumps, stop latch + get-going gesture, janked detection + freeze, rewind, \
gear-stat mix, drop start locator, ballistic landing prediction, prewind, angular-velocity filter, air rotation, \
touchdown snap + balance + bail/tumble, uber crash (any grab in its enter/loop phase) + recovery, grab gestures + \
SF_AirTricks, air scoring + boost meter + Tricky, boost thrust + snow spray, trick records + duplicates + combos + \
flow, rewind manager, the SF_Ride graph inside the step + root motion, grinds (surface analysis, path solver, grind \
state machine, grind steering, grind tricks + scoring), death volumes (trigger volumes, UpdateHealth, the carving's \
IsDead, the score manager's 4 s death timer, rewinding out of death), the tag runtime, the track start \
(BLN_Trigger_Go -> BLN_IsTrackStarted), SF_GameLoop's state_rewind (the recorded rider channels, a fresh SF_Ride at \
the hand-back), the board press (X / B: nose / tail on the ground, the rail press, blend, half boost, press scoring), \
the event (the drop's ui_drop + ghostdb rivals, the score manager's clock, 9 splits, the end volume, IsTrackFinished, \
the forced stop, AllowRaceEnd, results + place among the rivals), the wingsuit (R1 in the air once the race has \
properly started, SF_Ride's wingsuit nodes with their blends and windows, the WingsuitController's attitude PIDs, \
lift, roll turn and falloff, the retract's ballistic, the wear), survival (PlayerUpdate's health + armour, impact \
damage, regen, impact bails, health from tricks, rock-riding damage, temperature + sunlight, oxygen + tank on R1, \
lamp / goggles batteries, ice-axe wear, Lua UpdateHealth's freezing / asphyxia; the drop's required gear, the event's \
forced rider and their starting items' committed stats), the ice axe (deploy + chop layers, carving steering + grip \
on ice), avalanches (the ParticleSimFeature's slabs + sloughs, the carving's avalanche drag / wind / tumble / \
damage), tripping (TripTest, the hop, the rock bail), the impact re-entry (state_riding -> state_riding on \
BLN_TriggerImpact) and the touchdown's impact kicks (SpinImpactCurve, UseImpactTuning), the boundary's wind (VEC_Wind \
= the grind steering's + VEC_BoundaryWindCorrection), the carving's collision steering, the rewind channels' packing \
(the 32-bit rotation words, 10:10:10 translations, the playback's snapshot blend), bots (session::bots: the bot \
requests, GlobalRandom_Below's Mersenne twister, the catch-up clock, the bounce off a bot with the replayed velocity, \
the bot's sf_ghost_wipeout layer and its replay slowdown, the bots as the score manager's racers), the survive-to-death \
loop (session::survive: the heli-pickup outro, the score manager's states 4 / 5, LoopToTopReset with the lap's bonuses, \
the short intro again, the descent over the loops), the heli-jump finish (session::heli_jump: himeast1001's objectives \
— the helicopter to nis_end_heli_grab's start, the HeliGoToEnd override and its warp —, the jump assistance of the \
take-off (intercept: the quadratic-drag closed forms, the golden section, Newton on the discrete flight), the contact \
query on the airborne nodes, nis_end_heli_grab and BLN_AllowRaceEnd), the event's heli intro NIS (the short path: the door wait on the \
helicopter's Floor, GO, the jump's root motion, the hand-over to SF_Ride in the air, the race clock one execution \
later), the snow-start intros (the ready and go picks), the standard outro NIS (the win / neutral / lose reaction at \
nis_outro_locator, the camera-end object's vista after the cut), the delayed finish of the heli-jump drops (built, \
posted after the race end), the drop's helicopter in the intro and its flight after it (SF_Helicopter: \
HeliUpdateSceneOp's desired position over the height grid, HeliMovementController's PID, thrust and drag, the facing \
spring, HeliGoToEnd to the outro locator), the disc geotags (session::geotags: the drop's placement — collected ones \
skipped, the 100 m spacing, 25 kept at random —, the GeoCache controller's spin, bounce, touch, flight to the rider and \
pickup, GEOCACHE_ResultList into the results); port choices: the riding start of tests, the uber-crash landing's pivot \
blend, a stopped rider's grind veering, the finish message's exit, the medal rule, the trick flow left by a deploy, \
the sunlight ray headless (the viewer hands in its shadow read-back), the Explore drops' gear (the player's profile), \
a stopped rider's board angle, the helicopter's chase-target velocity source, the bots' and the geotags' seeds; not modelled: the long / \
medium cabin intros, the helicopter's tilt and heli tricks";
