use serde_derive::Deserialize;
use serde_inline_default::serde_inline_default;
use std::{fs, sync::OnceLock};
use structopt::StructOpt;

#[serde_inline_default]
#[derive(Debug, StructOpt, Deserialize)]
pub struct Cli {
    /// Reads the command line arguments from a supplied path as toml.
    #[structopt(long)]
    #[serde(default, skip_deserializing)]
    config: Option<String>,
    /// Path to the locally stored program data (LMDB).
    #[structopt(long, default_value = "data/pinner")]
    #[serde_inline_default("data/pinner".to_string())]
    pub data: String,
    /// The node admin endpoint this pinner manages subscriptions against.
    /// Loopback-only by convention; the pinner uses the node's
    /// `~/.samizdat/admin-token` for auth and must run as the same user.
    #[structopt(long, default_value = "http://localhost:4510")]
    #[serde_inline_default("http://localhost:4510".to_string())]
    pub node: String,
    /// The port on which to serve the pinner's HTTP control surface.
    #[structopt(long, default_value = "4512")]
    #[serde_inline_default(4512)]
    pub port: u16,
    /// Shared API key required on `X-Api-Key` for every `/pin*` request.
    /// V1 has a single key for the whole pinner; per-customer keys and
    /// Polygon receipt verification come in V1.5 / V2.
    #[structopt(long)]
    #[serde(default)]
    pub api_key: Option<String>,
    /// How often (in seconds) the expiry loop scans for expired pins.
    #[structopt(long, default_value = "60")]
    #[serde_inline_default(60)]
    pub expiry_tick_seconds: u64,
}

impl Cli {
    fn or_read_from_file(self) -> Result<Self, anyhow::Error> {
        let Some(config) = self.config else {
            return Ok(self);
        };

        let loaded: Self = toml::from_str(&fs::read_to_string(config)?)?;

        if loaded.config.is_some() {
            tracing::warn!("`config` variable set in config file. This has no effect");
        }

        Ok(loaded)
    }
}

static CLI: OnceLock<Cli> = OnceLock::new();

pub fn init_cli() -> Result<(), anyhow::Error> {
    let cli = Cli::from_args().or_read_from_file()?;
    tracing::debug!("Arguments from command line: {:#?}", cli);
    CLI.set(cli).ok();

    Ok(())
}

pub fn cli<'a>() -> &'a Cli {
    CLI.get().expect("cli was initialized")
}
