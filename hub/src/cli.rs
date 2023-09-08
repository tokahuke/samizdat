//! Command line interface for the Samizdat node.

use structopt::StructOpt;

use samizdat_common::address::{AddrResolutionMode, AddrToResolve, HubAddr};

/// The Samizdat Hub.
#[derive(StructOpt)]
pub struct Cli {
    /// Set logging level.
    #[structopt(env = "SAMIZDAT_VERBOSE", long, short = "v")]
    pub verbose: bool,
    /// The socket addresses for nodes to connect as clients.
    #[structopt(env = "SAMIZDAT_ADDRESSES", long, default_value = "[::]:4511/4512")]
    pub addresses: Vec<HubAddr>,
    /// Path to the locally stored program data.
    #[structopt(env = "SAMIZDAT_DATA", long, default_value = "data/hub")]
    pub data: String,
    /// Maximum number of simultaneous connections.
    #[structopt(env = "SAMIZDAT_MAX_CONNECTIONS", long, default_value = "2048")]
    pub max_connections: usize,
    /// Maximum number of _simultaneous_ resolutions per query.
    #[structopt(env = "SAMIZDAT_MAX_RESOLUTIONS_PER_QUERY", long, default_value = "12")]
    pub max_resolutions_per_query: usize,
    /// The maximum number of _simultaneous_ requests a node can make.
    #[structopt(env = "SAMIZDAT_MAX_QUERY_PER_NODE", long, default_value = "12")]
    pub max_queries_per_node: usize,
    /// The inverse of the interval that we delay if a node is requesting too many queries.
    /// (e.g., 2 => delay 500ms).
    #[structopt(env = "SAMIZDAT_MAX_QUERY_RATE_PER_NODE", long, default_value = "12")]
    pub max_query_rate_per_node: f64,
    /// The maximum number of candidates to return to the client.
    #[structopt(env = "SAMIZDAT_MAX_CANDIDATES", long, default_value = "3")]
    pub max_candidates: usize,
    /// Other servers to which to listen to.
    #[structopt(env = "SAMIZDAT_PARTNERS", long)]
    pub partners: Option<Vec<AddrToResolve>>,
    /// The mode of resolution to be used with domain names. Must be one of `ensure-ipv4`,
    /// `ensure-ipv6`, `prefer-ipv6`, `prefer-ipv4` or `use-both`. Note that the `prefer-*` options
    /// will resolve to the other IP version if no address is available for the current version.
    #[structopt(env = "SAMIZDAT_RESOLUTION_MODE", long, default_value = "use-both")]
    pub resolution_mode: AddrResolutionMode,
    /// The port for the monitoring http server.
    #[structopt(env = "SAMIZDAT_HTTP_PORT", long, default_value = "45180")]
    pub http_port: u16,
}
