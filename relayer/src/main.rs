use std::path::PathBuf;

use anyhow::{Context, Result};
use bridge_relayer::config::Config;
use bridge_relayer::relay::Relayer;
use bridge_relayer::tx::Signer;
use clap::{Parser, Subcommand};

/// Reference relayer for BridgeVault.sol.
#[derive(Parser)]
#[command(version, about, propagate_version = true)]
struct Cli {
    /// Path to the TOML config
    #[arg(short, long, global = true, default_value = "relayer.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check RPC connectivity, chain ids and the relayer balance
    Check {
        /// Also load the signing key and show its balance
        #[arg(long)]
        with_key: bool,
    },
    /// Scan for BridgeRequested events and relay them
    Run {
        /// Sign and send release() transactions (default is dry-run)
        #[arg(long)]
        execute: bool,
        /// Single pass, then exit (useful for cron and tests)
        #[arg(long)]
        once: bool,
    },
}

fn signer() -> Result<Signer> {
    let key = std::env::var("RELAYER_PRIVATE_KEY").context("RELAYER_PRIVATE_KEY is not set")?;
    Signer::from_hex(&key)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = Config::load(&cli.config)?;
    match cli.command {
        Command::Check { with_key } => {
            let s = if with_key { Some(signer()?) } else { None };
            Relayer::new(cfg, s)?.check()
        }
        Command::Run { execute, once } => {
            let s = if execute { Some(signer()?) } else { None };
            let mut relayer = Relayer::new(cfg, s)?;
            if !execute {
                eprintln!("dry-run: no transactions will be sent (pass --execute)");
            }
            loop {
                match relayer.tick() {
                    Ok(outcomes) if once => {
                        eprintln!("{} event(s) handled", outcomes.len());
                        return Ok(());
                    }
                    Ok(_) => {}
                    Err(e) if once => return Err(e),
                    Err(e) => eprintln!("error: {e:#} (retrying next tick)"),
                }
                std::thread::sleep(relayer.poll_interval());
            }
        }
    }
}
