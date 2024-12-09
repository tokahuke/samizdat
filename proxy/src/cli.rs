use serde_derive::Deserialize;
use std::{fs, sync::OnceLock};
use structopt::StructOpt;

#[derive(Debug, StructOpt, Deserialize)]
pub struct Cli {
    /// Reads the command line arguments from a supplied path as toml.
    #[structopt(long)]
    #[serde(default, skip_deserializing)]
    config: Option<String>,
    /// The port on which to serve the proxy. This only has effect when serving HTTP only.
    #[structopt(long)]
    #[serde(default)]
    pub port: Option<u16>,
    /// Whether to serve with HTTPS. This is meant for production only.
    #[structopt(long)]
    #[serde(default)]
    pub https: bool,
    /// The name of the domain that this proxy will serve (only applicable if HTTPS is
    /// set).
    #[structopt(long)]
    #[serde(default)]
    pub domain: Option<String>,
    /// The e-mail of the owned of the domain (this will be passed to `certbot`; only
    /// applicable if HTTPS is set).
    #[structopt(long)]
    #[serde(default)]
    pub owner: Option<String>,
}

impl Cli {
    fn or_read_from_file(self) -> Result<Self, anyhow::Error> {
        let Some(config) = self.config else {
            return Ok(self);
        };

        Ok(toml::from_str(&fs::read_to_string(config)?)?)
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
