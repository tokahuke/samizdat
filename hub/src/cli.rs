use structopt::StructOpt;

#[derive(StructOpt)]
pub struct Cli {
    #[structopt(env, long, default_value = "data/db")]
    pub db_path: String,
    /// Maximum number of simultaneous connections.
    #[structopt(env, long, default_value = "1024")]
    pub max_connections: usize,
    /// Maximum number of _simultaneous_ resolutions per query.
    #[structopt(env, long, default_value = "12")]
    pub max_resolutions_per_query: usize,
    /// The maximum number of _simultaneous_ queries a node can make.
    #[structopt(env, long, default_value = "4")]
    pub max_queries_per_node: usize,
    /// The inverse of the interval that we delay if a node is requesting too many queries
    /// (e.g., 2 => delay 500ms).
    #[structopt(env, long, default_value = "12")]
    pub max_query_rate_per_node: f64,
    /// The maximum number of candidates to return to the client.
    #[structopt(env, long, default_value = "1")]
    pub max_candidates: usize,
}
