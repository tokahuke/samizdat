//! Command line interface for the Samizdat node.

use std::path::PathBuf;
use structopt::StructOpt;

use samizdat_common::address::{AddrResolutionMode, AddrToResolve};

/// The Samizdat Client.
#[derive(Debug, StructOpt)]
pub struct Cli {
    /// Set logging level.
    #[structopt(short = "v")]
    pub verbose: bool,
    /// Path to the locally stored program data.
    #[structopt(env = "SAMIZDAT_DATA", long, default_value = "data/node")]
    pub data: PathBuf,
    /// The port on which to sever the local HTTP proxy. This is the port you will use to access in
    ///  your browser.
    #[structopt(env = "SAMIZDAT_PORT", long, default_value = "4510")]
    pub port: u16,
    /// (MB) The maximum size in bytes of the content that can be sent from a peer to this machine.
    #[structopt(env = "SAMIZDAT_MAX_CONTENT_SIZE", long, default_value = "1000")]
    pub max_content_size: usize,
    /// A list of hubs to which to connect.
    #[structopt(env = "SAMIZDAT_HUBS", long, default_value = "[::1]:4511")]
    pub hubs: Vec<AddrToResolve>,
    /// The mode of resolution to be used with domain names. Must be one of `ensure-ipv4`,
    /// `ensure-ipv6`, `prefer-ipv6`, `prefer-ipv4` or `use-both`. Note that the `prefer-*` options
    /// will resolve to the other IP version if no address is available for the current version.
    #[structopt(env = "SAMIZDAT_RESOLUTION_MODE", long, default_value = "use-both")]
    pub resolution_mode: AddrResolutionMode,
    /// The maximum number of hubs to be queried simultaneously per query.
    #[structopt(env = "SAMIZDAT_MAX_PARALLEL_HUBS", long, default_value = "3")]
    pub max_parallel_hubs: usize,
    /// (MB) The maximum total size of all cached files and _disposable_ files. Note that the total
    /// size may still exceed this value, since some of the allocated space is used to store
    /// data that is valuable to you.
    #[structopt(env = "SAMIZDAT_MAX_STORAGE", long, default_value = "1000")]
    pub max_storage: usize,
    /// The number of riddles to be sent on each query. This gives the maximum number of hops that a
    /// query can propagate inside a network, with 2 being the absolute minimum to get a result.
    #[structopt(env = "SAMIZDAT_RIDDLES_PER_QUERY", long, default_value = "6")]
    pub riddles_per_query: usize,
    /// The maximum number of simultaneous candidates (peers that have the content you queried) to
    /// accept when processing a query to the network.
    #[structopt(env = "SAMIZDAT_CONCURRENT_CANDIDATES", long, default_value = "4")]
    pub concurrent_candidates: usize,
}

/// The handle to the CLI parameters.
static mut CLI: Option<Cli> = None;

/// Initializes the [`CLI`] with the values from the command line.
pub fn init_cli() -> Result<(), crate::Error> {
    let cli = Cli::from_args();

    log::info!("Arguments from command line: {:#?}", cli);

    std::fs::create_dir_all(&cli.data)?;

    log::debug!("Initialized data folder");

    unsafe {
        CLI = Some(cli);
    }

    Ok(())
}

/// Returns a handle to the CLI arguments. Only use this after initialization.
pub fn cli<'a>() -> &'a Cli {
    unsafe { CLI.as_ref().expect("cli not initialized") }
}
