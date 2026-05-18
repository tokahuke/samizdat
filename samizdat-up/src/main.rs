//! samizdat-up - cross-platform installer + updater for the Samizdat daemons.
//!
//! Replaces the per-platform install scripts (shell + NSIS + brew formula
//! mosaic). One binary per (os, arch); same UX everywhere.

mod cli;
mod daemons;
mod fetch;
mod install;

use clap::Parser;

fn main() {
    let args = cli::Cli::parse();
    if let Err(err) = args.run() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}
