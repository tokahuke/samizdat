#![feature(try_blocks)]

mod access;
mod cli;
mod db;
mod http;
mod identity_dapp;
mod models;
mod system;
mod utils;
mod vacuum;

pub use samizdat_common::Error;

pub use cli::cli;

use std::sync::OnceLock;

use access::init_access_token;
use cli::init_cli;
use db::init_db;
use identity_dapp::init_identity_provider;
use samizdat_common::address::AddrResolutionMode;
use samizdat_common::db::writable_tx;
use system::Hubs;

/// The variable holding a list of all the connections to the hubs.
static HUBS: OnceLock<Hubs> = OnceLock::new();

/// Hubs the node connects to on first run when its hubs table is empty and
/// no `.default-hubs-seeded` marker exists in the data dir. Hardcoded to
/// get fresh installs out of the "no peers, content goes nowhere" hole.
/// Removing or changing entries here only affects nodes whose marker file
/// is absent (clean install, or operator manually removed the marker).
const DEFAULT_HUBS: &[(&str, AddrResolutionMode)] = &[
    ("testbed.hubfederation.com", AddrResolutionMode::UseBoth),
];

/// Inserts [`DEFAULT_HUBS`] into the node's hubs table the first time the
/// node starts. Idempotency is enforced by a marker file in the data dir,
/// NOT by checking whether the hubs table is empty: if it were the latter,
/// a user who deletes their last hub would have it re-seeded on next
/// restart, which is surprising. Operator can re-trigger seeding by
/// deleting the marker.
async fn maybe_seed_default_hubs() -> Result<(), crate::Error> {
    let marker = cli().data.join(".default-hubs-seeded");
    if marker.exists() {
        return Ok(());
    }

    for (address, resolution_mode) in DEFAULT_HUBS {
        let hub = models::Hub {
            address: (*address).to_owned(),
            resolution_mode: *resolution_mode,
        };
        let exists = samizdat_common::db::readonly_tx(|tx| {
            models::Hub::get(tx, &hub.address).map(|h| h.is_some())
        })?;
        if exists {
            continue;
        }
        tracing::info!("Seeding default hub {} ({:?})", hub.address, hub.resolution_mode);
        writable_tx(|tx| hub.insert(tx))?;
    }

    if let Err(err) = std::fs::write(&marker, b"") {
        tracing::warn!(
            "Could not write default-hubs marker {}: {err}. Defaults may be re-seeded on next \
             start.",
            marker.display(),
        );
    }
    Ok(())
}

/// Initiates [`HUBS`] by connecting to all hubs defined in the command line.
async fn init_hubs() -> Result<(), crate::Error> {
    maybe_seed_default_hubs().await?;
    let hubs = Hubs::init().await?;
    HUBS.set(hubs).ok();

    Ok(())
}

/// Retrieves a reference to the list of hubs. Needs to be called just after initialization.
pub fn hubs<'a>() -> &'a Hubs {
    HUBS.get().expect("hubs not initialized")
}

/// The entrypoint of the Samizdat node.
#[tokio::main]
async fn main() -> Result<(), crate::Error> {
    init_cli()?;

    // Init logger:
    samizdat_common::logger::init();

    tracing::info!(
        "Starting SAMIZDAT node in folder {:?}",
        cli().data.canonicalize()?
    );

    // Init resources:
    init_db::<crate::db::Table>(&cli().data.to_string_lossy())?;
    init_access_token()?;
    init_identity_provider()?;

    // Recover from any chunks left behind by previous crashed imports. Must run BEFORE
    // any task that calls `ObjectRef::do_import` is spawned; otherwise we'd race a
    // legitimate in-flight import.
    crate::vacuum::sweep_crash_leaked_chunks()?;

    init_hubs().await?;

    // Start vacuum:
    tokio::spawn(crate::vacuum::run_vacuum_daemon());

    // Run public server:
    http::serve().await?;

    // Exit:
    Ok(())
}
