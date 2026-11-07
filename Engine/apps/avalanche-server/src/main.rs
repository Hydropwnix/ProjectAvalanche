use std::path::PathBuf;
use std::process::ExitCode;

use avalanche_server::data::setup::{self, Source};
use avalanche_server::{ServerOptions, config, keys};
use clap::{Parser, Subcommand};

/// Project Avalanche's dedicated multiplayer server.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// The configuration file (missing: the defaults).
    #[arg(long, global = true, default_value = "server.toml")]
    config: PathBuf,
    /// The data folder: the server's keys, logs and the rules subset of your disc.
    #[arg(long, global = true, default_value = "data")]
    data: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server (the default).
    Run {
        /// Stop when stdin ends (a program that starts the server and holds its stdin, as the launcher's Host does, so
        /// the server never outlives it).
        #[arg(long)]
        stop_with_input: bool,
    },
    /// Make the data folder's keys (refused if a key file exists: keys are never replaced).
    Keygen {
        /// Make an operator key at this file instead (for the remote console; its public key goes into acl.toml).
        #[arg(long, value_name = "FILE")]
        operator: Option<PathBuf>,
    },
    /// Take the rules subset (the files a server reads) from your own SSX (2012) disc image into the data folder (the
    /// resources folder the configuration names there is left as it is).
    Setup {
        /// The disc image (an Xbox 360 ISO of the retail disc).
        #[arg(required_unless_present = "from_extraction", conflicts_with = "from_extraction")]
        iso: Option<PathBuf>,
        /// Take the members from the game's own data folder (the launcher's GameData, with big/) instead of an image.
        #[arg(long, value_name = "GAME_DATA")]
        from_extraction: Option<PathBuf>,
        /// The folder to write (default: the data folder, --data).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Optional classes besides the rules: collision, ghosts, verify (comma separated). Maps (round net8): a map
        /// in a round mode needs verify (the start is checked with a rider's animation), a layout needs collision (its
        /// base drop's terrain).
        #[arg(long, value_delimiter = ',')]
        with: Vec<String>,
    },
    /// Check the configuration, the data folder's keys and its rules subset (every member and hash), and say what the
    /// server would do.
    Check,
    /// Run a console command on a server with an operator key (the remote console) and print its answer.
    Rcon {
        /// The server: host[:port] or an address (the game port by default).
        address: String,
        /// The operator key (made by `keygen --operator`).
        #[arg(long, value_name = "FILE")]
        key: PathBuf,
        /// The server's identity key (hex), instead of the pin file beside the operator key.
        #[arg(long, value_name = "HEX")]
        server_key: Option<String>,
        /// Pin the key the server offers when its address is not pinned yet.
        #[arg(long)]
        accept_new: bool,
        /// The command and its arguments.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run { stop_with_input: false }) {
        Command::Run { stop_with_input } => {
            if stop_with_input {
                avalanche_server::console::stop_at_end_of_input();
            }
            let config = match config::load(&cli.config) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(2);
                }
            };
            let options = ServerOptions {
                config,
                data_dir: cli.data,
                console: true,
                logging: true,
                log_to_file: true,
                signals: true,
                conditions: None,
                relay_rates: None,
                extend: None,
                rules: avalanche_server::rules::RulesSource::Config,
            };
            match avalanche_server::run(options) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Keygen { operator: Some(path) } => match avalanche_server::operation::keys::generate(&path) {
            Ok(secret) => {
                println!("made {}", path.display());
                println!("operator key {}", secret.public().to_hex());
                println!("give it rights on a server: acl add <group> operator.{}", secret.public().to_hex());
                println!("its id in the server's log and in bans.toml: operator {}", secret.public().id().short());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(2)
            }
        },
        Command::Rcon { address, key, server_key, accept_new, command } => {
            let options = avalanche_server::operation::rcon_client::RconOptions {
                address,
                key,
                server_key,
                accept_new,
                command: command.join(" "),
            };
            run_rcon(&options)
        }
        Command::Keygen { operator: None } => match keys::generate(&cli.data) {
            Ok(k) => {
                for path in &k.created {
                    println!("made {}", path.display());
                }
                println!("identity key {}", k.identity_key().to_hex());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(2)
            }
        },
        Command::Setup { iso, from_extraction, out, with } => {
            let source = match (iso, from_extraction) {
                (Some(iso), None) => Source::Iso(iso),
                (None, Some(dir)) => Source::Extraction(dir),
                _ => {
                    eprintln!("give a disc image or --from-extraction <game data folder>");
                    return ExitCode::from(2);
                }
            };
            run_setup(&source, &out.unwrap_or(cli.data), &cli.config, &with)
        }
        Command::Check => {
            let ok = avalanche_server::check::run(&cli.config, &cli.data, &mut |line| println!("{line}"));
            if ok { ExitCode::SUCCESS } else { ExitCode::from(2) }
        }
    }
}

fn run_rcon(options: &avalanche_server::operation::rcon_client::RconOptions) -> ExitCode {
    use ssx_net_proto::rcon::RconAnswer;
    match avalanche_server::operation::rcon_client::run(options) {
        Ok((notes, answer)) => {
            for note in notes {
                eprintln!("{note}");
            }
            match answer {
                RconAnswer::Ran { ok, lines } => {
                    for line in lines {
                        ssx_core::println!("{line}");
                    }
                    if ok { ExitCode::SUCCESS } else { ExitCode::from(1) }
                }
                RconAnswer::Refused(r) => {
                    let wait = if r.retry_after_secs > 0 {
                        format!(" (try again in {} s)", r.retry_after_secs)
                    } else {
                        String::new()
                    };
                    ssx_core::eprintln!("refused, {:?}: {}{wait}", r.code, r.text);
                    ExitCode::from(3)
                }
            }
        }
        Err(e) => {
            ssx_core::eprintln!("{}", ssx_core::text::message(&e));
            ExitCode::from(2)
        }
    }
}

fn run_setup(source: &Source, out: &std::path::Path, config_path: &std::path::Path, with: &[String]) -> ExitCode {
    use avalanche_server::data::Class;
    let resources = match config::load(config_path) {
        Ok(c) => c.resources.folder,
        Err(e) => {
            eprintln!("setup refused: {e}");
            return ExitCode::from(2);
        }
    };
    let mut classes = std::collections::BTreeSet::from([Class::Rules]);
    for w in with {
        match Class::from_option(w) {
            Some(c) => {
                classes.insert(c);
            }
            None => {
                ssx_core::eprintln!("--with {}: collision, ghosts or verify", ssx_core::text::quoted(w));
                return ExitCode::from(2);
            }
        }
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut last = std::time::Instant::now();
    let mut progress = |p: &ssx_assets::setup::Progress| {
        if last.elapsed().as_millis() >= 500 {
            last = std::time::Instant::now();
            eprintln!("  {} {}/{} files", p.archive, p.files_done, p.files_total);
        }
    };
    match setup::run(source, out, &resources, &classes, &cancel, &mut progress) {
        Ok(r) => {
            println!(
                "{}: {} members, {:.1} MB ({} written, {} kept) in {:.1} s; {}",
                out.display(),
                r.members,
                r.bytes as f64 / 1e6,
                r.written,
                r.kept,
                r.seconds,
                r.pressing
            );
            println!(
                "  checked: {} against the known disc's sizes and hashes, {} against the disc's MD5 lists, {} against the extraction's index",
                r.known_checked, r.md5_checked, r.index_checked
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("setup refused: {e}");
            ExitCode::from(2)
        }
    }
}
