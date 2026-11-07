use std::path::{Path, PathBuf};
use std::process::ExitCode;

use avalanche_master::{MasterOptions, blocklists::Blocklists, config};
use clap::{Parser, Subcommand};

/// Project Avalanche's master server: the list of public multiplayer servers.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// The configuration file (missing: the defaults).
    #[arg(long, global = true, default_value = "master.toml")]
    config: PathBuf,
    /// The data folder: the logs.
    #[arg(long, global = true, default_value = "data")]
    data: PathBuf,
}

#[derive(Subcommand)]
enum Command {
    /// Run the master (the default).
    Run,
    /// Check the configuration, the blocklists and the TLS files, and say what the master would do.
    Check,
    /// Ask the running master on its port whether it is healthy (Docker's and Coolify's health check): exit 0 when it
    /// is, 1 when it is not (2 is Docker's reserved code).
    Health,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = match config::load(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", ssx_core::text::Text::new(&e));
            return ExitCode::from(if matches!(cli.command, Some(Command::Health)) { 1 } else { 2 });
        }
    };
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => {
            let options = MasterOptions {
                config,
                data_dir: cli.data,
                console: true,
                logging: true,
                log_to_file: true,
                signals: true,
            };
            match avalanche_master::run(options) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{}", ssx_core::text::Text::new(&e));
                    ExitCode::FAILURE
                }
            }
        }
        Command::Check => check(&cli.config, &config),
        Command::Health => match avalanche_master::health::check(&config) {
            Ok(addr) => {
                println!("healthy: {addr} answered GET /v1/health");
                ExitCode::SUCCESS
            }
            Err(why) => {
                println!("unhealthy: {}", ssx_core::text::Text::new(&why));
                ExitCode::from(1)
            }
        },
    }
}

fn mode(c: &config::Config) -> Vec<String> {
    let trusted = avalanche_master::trusted_list(&c.trusted);
    let rule = avalanche_master::forwarded_rule();
    let mut lines = vec![match (c.tls.is_some(), c.trusted.is_empty()) {
        (true, true) => "mode: direct TLS (the master serves HTTPS itself; no proxy)".to_string(),
        (false, false) => format!("mode: plain HTTP behind trusted proxies {trusted} (they terminate TLS); {rule}"),
        (true, false) => format!(
            "mode: HTTPS behind trusted proxies {trusted} (they connect over TLS; a TCP pass-through adds no \
             X-Forwarded-For and every request through it is refused); {rule}"
        ),
        (false, true) => "mode: plain HTTP with no proxy".to_string(),
    }];
    if c.tls.is_none() && c.trusted.is_empty() && !c.bind.is_loopback() {
        lines.push(format!(
            "warning: plain HTTP on {} with no proxy: a master on the Internet serves HTTPS ([tls]) or sits behind a \
             TLS-terminating proxy ([proxy] trusted); plain HTTP is for a LAN or test master",
            c.addr()
        ));
    }
    for net in c.trusted.networks() {
        if !avalanche_master::address::wholly_not_public(net) {
            lines.push(format!(
                "warning: the trusted network {net} holds addresses the Internet routes to: trust only your proxies' \
                 own network, or a host there can name any client"
            ));
        }
    }
    if !c.trusted.is_empty() {
        let route = avalanche_master::route::default_gateways();
        lines.extend(avalanche_master::route::gateway_lines(&route, &c.trusted));
    }
    lines
}

fn check(path: &Path, c: &config::Config) -> ExitCode {
    let mut ok = true;
    match &c.path {
        Some(p) => println!("{}: valid", p.display()),
        None => println!("{}: missing, the defaults are used", path.display()),
    }
    println!("  address {} ({})", c.addr(), if c.tls.is_some() { "HTTPS" } else { "plain HTTP" });
    mode(c).iter().for_each(|l| println!("  {l}"));
    println!("  at most {} servers, {} per address", c.max_servers, c.max_per_address);
    println!(
        "  {}",
        if c.allow_private { "private addresses allowed (a LAN or test master)" } else { "public addresses only" }
    );
    match Blocklists::load(c) {
        Ok(b) => println!("  blocklists: {} server keys, {} resources", b.servers.len(), b.resources.len()),
        Err(e) => {
            println!("  {}", ssx_core::text::Text::new(&e));
            ok = false;
        }
    }
    if let Some(tls) = &c.tls
        && let Err(e) = avalanche_master::tls::acceptor(tls)
    {
        println!("  TLS: {}", ssx_core::text::Text::new(&e));
        ok = false;
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::from(2) }
}
