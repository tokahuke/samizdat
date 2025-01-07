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
    /// Path to the locally stored program data.
    #[structopt(long, default_value = "data/proxy")]
    #[serde_inline_default("data/proxy".to_string())]
    pub data: String,
    /// The node to which to connect to. Defaults to localhost:4510.
    #[structopt(long, default_value = "http://localhost:4510")]
    #[serde_inline_default("http://localhost:4510".to_string())]
    pub node: String,
    /// Whether to serve with HTTPS. This is meant for production only.
    #[structopt(long)]
    #[serde(default)]
    pub https: bool,
    /// The port on which to serve the proxy. This defaults to 443 when HTTPS is enabled
    /// and to 8080 when HTTP is enabled.
    #[structopt(long)]
    #[serde(default)]
    pub port: Option<u16>,
    /// The port on which to serve the HTTP-to-HTTPS redirector. This defaults to 80 (only
    /// applicable if HTTPS is set).
    #[structopt(long)]
    #[serde(default)]
    pub http_port: Option<u16>,
    /// The name of the domain that this proxy will serve (only applicable if HTTPS is
    /// set).
    #[structopt(long)]
    #[serde(default)]
    pub domain: Option<String>,
    /// The e-mail of the owner of the domain (only applicable if HTTPS is set).
    #[structopt(long)]
    #[serde(default)]
    pub owner: Option<String>,
    /// The directory that provides certificates. Defaults to the Let's Encrypt v2 ACME
    /// directory (only applicable if HTTPS is set).
    #[structopt(long, default_value = "https://acme-v02.api.letsencrypt.org/directory")]
    #[serde_inline_default("https://acme-v02.api.letsencrypt.org/directory".to_string())]
    pub acme_directory: String,
    /// The number of pages between which to show the "Get Samizdat!" modal.
    #[structopt(long, default_value = "10")]
    #[serde_inline_default(10)]
    pub show_modal_every: u16,
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
    pub fn domain(&self) -> Result<&str, anyhow::Error> {
        let Some(domain) = self.domain.as_ref() else {
            anyhow::bail!("missing domain parameter")
        };

        Ok(domain)
    }

    pub fn owner(&self) -> Result<&str, anyhow::Error> {
        let Some(owner) = self.owner.as_ref() else {
            anyhow::bail!("missing owner parameter")
        };

        Ok(owner)
    }
}

static CLI: OnceLock<Cli> = OnceLock::new();

pub fn init_cli() -> Result<(), anyhow::Error> {
    let cli = Cli::from_args().or_read_from_file()?;
    tracing::info!("Arguments from command line: {:#?}", cli);
    CLI.set(cli).ok();

    Ok(())
}

pub fn cli<'a>() -> &'a Cli {
    CLI.get().expect("cli was initialized")
}
