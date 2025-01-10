//! The `Samizdat.toml` manifest format.
//!
//! This module handles the configuration files for Samizdat projects, managing both public
//! (`Samizdat.toml`) and private (`.Samizdat.priv`) manifests. These files store project
//! metadata, build settings, and cryptographic keys for series management.

use askama::Template;
use serde_derive::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::{fs, io};

use samizdat_common::{Key, PrivateKey};

use crate::api;

/// Template for generating new Samizdat.toml files.
#[derive(askama::Template)]
#[template(path = "Samizdat.toml.txt")]
pub struct ManifestTemplate<'a> {
    /// Name of the series owner
    pub name: &'a str,
    /// Public key for the series
    pub public_key: &'a Key,
    /// Time-to-live duration for series content
    pub ttl: &'a str,
}

/// Configuration for a Samizdat project.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    /// Series-specific configuration
    pub series: Series,
    /// Debug environment settings
    pub debug: Debug,
    /// Build process configuration
    pub build: Build,
}

impl Manifest {
    /// Possible filenames for the manifest, in order of preference.
    const FILENAME_HIERARCHY: [&'static str; 4] = [
        "./Samizdat.toml",
        "./Samizdat.tml",
        "./samizdat.toml",
        "./samizdat.tml",
    ];

    /// Attempts to find and load an existing manifest file. Returns `None` if no
    /// manifest is found.
    pub fn find_opt() -> Result<Option<Manifest>, anyhow::Error> {
        for filename in Manifest::FILENAME_HIERARCHY {
            match fs::read_to_string(filename) {
                Ok(contents) => return Ok(Some(toml::from_str(&contents)?)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }

        Ok(None)
    }

    /// Creates a new manifest and associated debug keypair.
    ///
    /// # Arguments
    /// * `name` - The name of the series owner
    pub async fn create(name: &str) -> Result<(Manifest, PrivateKey), anyhow::Error> {
        if Manifest::find_opt()?.is_some() {
            anyhow::bail!("`Samizdat.toml` already exists.");
        }

        let response = api::post_series_owner(api::PostSeriesOwnerRequest {
            series_owner_name: name,
            keypair: None,
            is_draft: false,
        })
        .await?;

        let rendered = crate::manifest::ManifestTemplate {
            name,
            public_key: &Key::from(response.keypair.verifying_key()),
            ttl: &humantime::format_duration(response.default_ttl).to_string(),
        }
        .render()
        .expect("can render");

        fs::write("./Samizdat.toml", rendered)?;
        let manifest = toml::from_str(&fs::read_to_string("./Samizdat.toml")?)?;

        Ok((
            manifest,
            PrivateKey::from(response.keypair.to_scalar_bytes()),
        ))
    }

    /// Executes the build process according to manifest configuration.
    pub fn run_build(&self, is_release: bool) -> Result<(), anyhow::Error> {
        self.build.run(&self.series.public_key, is_release)
    }
}

/// Series-specific configuration settings.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Series {
    /// Name of the series
    pub name: String,
    /// Public key for the series
    pub public_key: String,
    /// Optional time-to-live duration for series content
    pub ttl: Option<String>,
}

/// Debug environment configuration.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Debug {
    /// Series owner name used in debug mode
    pub name: String,
}

/// Returns the default shell path.
///
/// Attempts to get the shell from the SHELL environment variable, falling back to
/// "/bin/sh" if not set.
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

/// Build process configuration.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Build {
    /// Base directory where Samizdat will read the produced content and create a
    /// new edition of the series.
    pub base: PathBuf,
    /// Command to run for release builds
    pub run: Option<String>,
    /// Command to run for debug builds
    pub run_debug: Option<String>,
    /// Shell to use for running commands
    #[serde(default = "default_shell")]
    pub shell: String,
}

impl Build {
    /// Executes the build process with the specified configuration.
    pub fn run(&self, public_key: &str, is_release: bool) -> Result<(), anyhow::Error> {
        let script = if is_release {
            self.run.as_ref()
        } else {
            self.run.as_ref().or(self.run_debug.as_ref())
        };
        let mut command = Command::new(&self.shell);
        command
            .arg("-c")
            .arg(script.map(String::as_str).unwrap_or_default())
            .env("SAMIZDAT_PUBLIC_KEY", public_key)
            .env("SAMIZDAT_RELEASE", if is_release { "release" } else { "" });

        println!("Running {:?}", command);

        let status = command.spawn()?.wait()?;

        if status.success() {
            Ok(())
        } else {
            Err(anyhow::anyhow!(
                "bad exit status for run command: {}",
                status
            ))
        }
    }
}

/// Template for generating new .Samizdat.priv files.
#[derive(askama::Template)]
#[template(path = "Samizdat.priv.txt")]
pub struct PrivateManifestTemplate<'a> {
    /// Optional production private key
    pub private_key: Option<&'a PrivateKey>,
    /// Debug environment private key
    pub private_key_debug: &'a PrivateKey,
    /// Debug environment public key
    pub public_key_debug: &'a Key,
}

/// Private configuration for a Samizdat project.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PrivateManifest {
    /// Optional production private key
    pub private_key: Option<String>,
    /// Debug environment private key
    pub private_key_debug: String,
    /// Debug environment public key
    pub public_key_debug: String,
}

impl PrivateManifest {
    /// Possible filenames for the private manifest, in order of preference.
    const FILENAME_HIERARCHY: [&'static str; 1] = ["./.Samizdat.priv"];

    /// Attempts to find and load an existing private manifest file. Returns `None` if no
    pub fn find_opt() -> Result<Option<PrivateManifest>, anyhow::Error> {
        for filename in PrivateManifest::FILENAME_HIERARCHY {
            match fs::read_to_string(filename) {
                Ok(contents) => return Ok(Some(toml::from_str(&contents)?)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }

        Ok(None)
    }

    /// Creates a new private manifest with the specified keys.
    ///
    /// # Arguments
    /// * `debug_name` - The name of the series owner
    /// * `private_key` - The private key for the series owner
    pub async fn create(
        debug_name: &str,
        private_key: Option<&PrivateKey>,
    ) -> Result<PrivateManifest, anyhow::Error> {
        if PrivateManifest::find_opt()?.is_some() {
            anyhow::bail!("`.Samizdat.priv` already exists.");
        }

        let response = api::post_series_owner(api::PostSeriesOwnerRequest {
            series_owner_name: debug_name,
            keypair: None,
            is_draft: true,
        })
        .await?;

        let rendered_private = crate::manifest::PrivateManifestTemplate {
            private_key,
            private_key_debug: &PrivateKey::from(response.keypair.to_scalar_bytes()),
            public_key_debug: &Key::from(response.keypair.verifying_key()),
        }
        .render()
        .expect("can render");

        fs::write("./.Samizdat.priv", rendered_private)?;
        let manifest = toml::from_str(&fs::read_to_string("./.Samizdat.priv")?)?;

        Ok(manifest)
    }
}
