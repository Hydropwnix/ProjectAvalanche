use ssx_movie::isolate;
use ssx_movie::isolate_os::Seal;

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod linux;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
use linux as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(not(any(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"), windows)))]
mod platform {
    pub fn attempt(_: &str, _: &str) -> i32 {
        super::not_stopped("this system has no sandbox")
    }
}

const ATTEMPT: &[u8] = b"SSX-SANDBOX-ATTEMPT\0";
const NOT_STOPPED: i32 = 97;

fn main() {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next()) {
        (None, _) => probe(),
        (Some(m), None) if m == "--measure" => isolate::serve_measured(),
        _ => {
            eprintln!("usage: avalanche-movie-probe [--measure] (started by the sandbox proof only)");
            std::process::exit(2)
        }
    }
}

fn probe() -> ! {
    let mut s = match isolate::enter_child(Seal::Enforce) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    };
    let code = match s.read_unit() {
        Ok(Some((_, unit))) if unit.starts_with(ATTEMPT) => {
            let rest = &unit[ATTEMPT.len()..];
            let (name, arg) =
                rest.iter().position(|&b| b == 0).map_or((rest, &[][..]), |k| (&rest[..k], &rest[k + 1..]));
            platform::attempt(&String::from_utf8_lossy(name), &String::from_utf8_lossy(arg))
        }
        Ok(first) => match s.serve_from(first) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Err(e) => {
            eprintln!("{e}");
            1
        }
    };
    std::process::exit(code)
}

fn not_stopped(what: &str) -> i32 {
    eprintln!("NOT STOPPED: {what}");
    NOT_STOPPED
}
