use std::io::Write as _;
use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use ssx_physics::rider::RideState;
use ssx_physics::rider::grind::measure::{path_up, rail_under};
use ssx_physics::rider::grind::state::RAIL_MASK;
use ssx_physics::{KMH_PER_M_PER_TICK, SIM_HZ};

use super::Ride;
use crate::camera::FlyCam;
use crate::rider::skin::RiderRig;

#[derive(Resource)]
pub struct GrindStrip {
    dir: PathBuf,
    every: Option<u64>,
    tail: u64,
    range: Option<(u64, u64)>,
    crop: Option<(u32, u32)>,
    exit: Option<u64>,
    left: u64,
    logged: Option<u64>,
    seen: (u64, u64),
    live_frames: u64,
    log: std::io::BufWriter<std::fs::File>,
}

impl GrindStrip {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("SSX_RIDE_STRIP")?);
        let num = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
        let pair = |name: &str| {
            let v = std::env::var(name).ok()?;
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?))
        };
        std::fs::create_dir_all(&dir).map_err(|e| warn!("SSX_RIDE_STRIP {}: {e}", dir.display())).ok()?;
        let file = std::fs::File::create(dir.join("strip.tsv")).map_err(|e| warn!("SSX_RIDE_STRIP: {e}")).ok()?;
        let mut log = std::io::BufWriter::new(file);
        let _ = writeln!(
            log,
            "step\tt\tstate\tgs\tgrinding\tsuper\tkmh\ttx\tty\ttz\tbx\tby\tbz\tboard_lat\tboard_height\ttraj_height\t\
             board_yaw\tboard_angle\thips_height\tlean\tcam_dist\tcam_up\tshot\tclips\tframe\tphase\tsub\tfrac\t\
             dx\tdy\tdz\tcx\tcy\tcz\tcfx\tcfy\tcfz\tbvx\tbvy\tbvz\thvx\thvy\thvz\tgauto\tgl2"
        );
        Some(Self {
            dir,
            every: num("SSX_RIDE_STRIP_EVERY").map(|n| n.max(1)),
            tail: num("SSX_RIDE_STRIP_TAIL").unwrap_or(8),
            range: pair("SSX_RIDE_STRIP_RANGE"),
            crop: pair("SSX_RIDE_STRIP_CROP").map(|(w, h)| (w as u32, h as u32)),
            exit: num("SSX_RIDE_STRIP_EXIT"),
            left: 0,
            logged: None,
            seen: (0, u64::MAX),
            live_frames: 0,
            log,
        })
    }
}

pub fn capture(
    mut commands: Commands,
    ride: Res<Ride>,
    rig: Option<Res<RiderRig>>,
    strip: Option<ResMut<GrindStrip>>,
    cameras: Query<&Transform, With<FlyCam>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut strip) = strip else { return };
    let live = ride.live_clock();
    let (frame, phase, sub) = match ride.drawn_frame() {
        Some(f) => (u64::from(f.index), f.phase, f.sub),
        None if live => {
            strip.live_frames += 1;
            (strip.live_frames - 1, 0, 0)
        }
        None => return,
    };
    if live && strip.exit.is_some_and(|n| ride.steps >= n) {
        exit.write(AppExit::Success);
    }
    if strip.seen == (ride.steps, frame) || ride.steps == 0 {
        return;
    }
    let new_step = ride.steps != strip.seen.0;
    strip.seen = (ride.steps, frame);
    let (s, gc) = (&ride.state, &ride.state.grind.carving);
    let active = match strip.range {
        Some((a, b)) => (a..=b).contains(&ride.steps),
        None => (s.state == RideState::Riding && gc.state != 0) || gc.is_grinding,
    };
    if active {
        strip.left = strip.tail + 1;
    } else if new_step && strip.left > 0 {
        strip.left -= 1;
    }
    if strip.left == 0 {
        strip.logged = None;
        return;
    }
    let logged = strip.logged.map_or(0, |n| n + 1);
    strip.logged = Some(logged);
    let every = strip.every.unwrap_or(if sub == 1 { 4 } else { 1 });
    let shot = !live && logged.is_multiple_of(every);
    let (p, v) = (s.traj, s.velocity);
    let n = path_up(v);
    let mask = if gc.super_grind { RAIL_MASK } else { 0 };
    let c = &ride.world.collision;
    let anim = rig.as_ref().map(|r| &r.anim);
    let board = anim.and_then(|a| a.drawn_joint("Board"));
    let probe = board.and_then(|b| rail_under(c, b.p, n, v, mask));
    let traj_probe = rail_under(c, p, n, v, mask);
    let flat = |x: Vec3| (x - n * x.dot(n)).normalize_or_zero();
    let yaw = anim
        .and_then(|a| a.drawn_axis("Board", Vec3::X))
        .map(|axis| flat(axis).dot(flat(v)).abs().clamp(0.0, 1.0).acos().to_degrees());
    let (hips, head) = (anim.and_then(|a| a.drawn_joint("Hips")), anim.and_then(|a| a.drawn_joint("Head")));
    let lean = hips.zip(head).map(|(h, t)| (t.p - h.p).normalize_or_zero().dot(n).clamp(-1.0, 1.0).acos().to_degrees());
    let d = anim.and_then(|a| a.drawn_joint("AITrajectory")).map_or(ride.render.traj, |j| j.p);
    let cam = cameras.iter().next();
    let view = |x: Option<Vec3>| x.zip(cam).map_or(Vec3::NAN, |(x, t)| t.rotation.inverse() * (x - t.translation));
    let (cp, cf) = cam.map_or((Vec3::NAN, Vec3::NAN), |t| (t.translation, *t.forward()));
    let (bv, hv) = (view(board.map(|b| b.p)), view(hips.map(|h| h.p)));
    let num = |x: Option<f32>| x.map_or("-".to_string(), |x| format!("{x:.4}"));
    let state = match s.state {
        RideState::Riding => "R",
        RideState::Airborne => "A",
        RideState::Jump => "J",
        _ => "W",
    };
    let b = board.map_or(Vec3::NAN, |b| b.p);
    let _ = write!(
        strip.log,
        "{}\t{:.4}\t{state}\t{}\t{}\t{}\t{:.2}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\t{}\t{}\t{:.1}\t{}\t{}\t{}\t{}\t{}\t{}",
        ride.steps,
        ride.steps as f32 / SIM_HZ,
        gc.state,
        u8::from(gc.is_grinding),
        u8::from(gc.super_grind),
        s.speed() * KMH_PER_M_PER_TICK,
        p.x,
        p.y,
        p.z,
        b.x,
        b.y,
        b.z,
        num(probe.map(|r| r.crown_lateral)),
        num(probe.map(|r| r.crown_gap)),
        num(traj_probe.map(|r| r.crown_gap)),
        num(yaw),
        gc.board_angle.to_degrees(),
        num(hips.map(|h| (h.p - d).dot(n))),
        num(lean),
        num(cam.map(|t| t.translation.distance(d))),
        num(cam.map(|t| (t.translation - d).dot(Vec3::Y))),
        u8::from(shot),
        anim.map_or(String::new(), |a| a.label()),
    );
    let _ = write!(strip.log, "\t{frame}\t{phase}\t{sub}\t{:.4}", ride.render_frac());
    for x in [d, cp, cf, bv, hv] {
        let _ = write!(strip.log, "\t{:.4}\t{:.4}\t{:.4}", x.x, x.y, x.z);
    }
    let _ = writeln!(strip.log, "\t{}\t{}", u8::from(gc.auto_grind), u8::from(s.grind.pad_l2));
    let _ = strip.log.flush();
    if shot {
        let name = if sub == 1 { format!("step_{:04}.png", ride.steps) } else { format!("frame_{frame:05}.png") };
        let (path, crop) = (strip.dir.join(name), strip.crop);
        commands.spawn(Screenshot::primary_window()).observe(move |captured: On<ScreenshotCaptured>| {
            match captured.image.clone().try_into_dynamic() {
                Ok(image) => {
                    let half = image.thumbnail(image.width() / 2, image.height() / 2);
                    let half = match crop {
                        Some((w, h)) => {
                            let (w, h) = (w.min(half.width()), h.min(half.height()));
                            half.crop_imm((half.width() - w) / 2, (half.height() - h) / 2, w, h)
                        }
                        None => half,
                    };
                    if let Err(e) = half.to_rgb8().save(&path) {
                        warn!("SSX_RIDE_STRIP: {}: {e}", path.display());
                    }
                }
                Err(e) => warn!("SSX_RIDE_STRIP: {e}"),
            }
        });
    }
}
