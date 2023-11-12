//! The `Samizdat.toml` manifest format.
//!

use askama::Template;
use serde_derive::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::{fs, io};

use samizdat_common::{Key, PrivateKey};

use crate::api;

#[derive(askama::Template)]
#[template(path = "Samizdat.toml.txt")]
pub struct ManifestTemplate<'a> {
    pub name: &'a str,
    pub public_key: &'a Key,
    pub ttl: &'a str,
    pub debug_name: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    pub series: Series,
    pub debug: Debug,
    pub build: Build,
}

impl Manifest {
    const FILENAME_HIERARCHY: [&'static str; 4] = [
        "./Samizdat.toml",
        "./Samizdat.tml",
        "./samizdat.toml",
        "./samizdat.tml",
    ];

    pub fn find_opt() -> Result<Option<Manifest>, anyhow::Error> {
        for filename in Manifest::FILENAME_HIERARCHY {
            match fs::read(filename) {
                Ok(contents) => return Ok(Some(toml::from_slice(&contents)?)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }

        Ok(None)
    }

    /// Creates a new manifest and associated debug keypair, given debug series owner name and
    /// optionally production private key.
    pub async fn create(name: &str) -> Result<(Manifest, PrivateKey), anyhow::Error> {
        if Manifest::find_opt()?.is_some() {
            anyhow::bail!("`Samizdat.toml` already exists.");
        }

        let debug_name = format!("{}-debug", name);

        let response = api::post_series_owner(api::PostSeriesOwnerRequest {
            series_owner_name: name,
            keypair: None,
            is_draft: false,
        })
        .await?;

        let rendered = crate::manifest::ManifestTemplate {
            name,
            public_key: &Key::from(response.keypair.public),
            ttl: &humantime::format_duration(response.default_ttl).to_string(),
            debug_name: &debug_name,
        }
        .render()
        .expect("can render");

        fs::write("./Samizdat.toml", rendered)?;
        let manifest = toml::from_str(&fs::read_to_string("./Samizdat.toml")?)?;

        Ok((manifest, PrivateKey::from(response.keypair.secret)))
    }

    pub fn run_build(&self, is_release: bool) -> Result<(), anyhow::Error> {
        self.build.run(&self.series.public_key, is_release)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Series {
    pub name: String,
    pub public_key: String,
    pub ttl: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Debug {
    pub name: String,
}

#[cfg(not(target_os = "darwin"))]
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Build {
    pub base: PathBuf,
    pub run: Option<String>,
    pub run_debug: Option<String>,
    #[serde(default = "default_shell")]
    pub shell: String,
}

impl Build {
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

#[derive(askama::Template)]
#[template(path = "Samizdat.priv.txt")]
pub struct PrivateManifestTemplate<'a> {
    pub private_key: Option<&'a PrivateKey>,
    pub private_key_debug: &'a PrivateKey,
    pub public_key_debug: &'a Key,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PrivateManifest {
    pub private_key: Option<String>,
    pub private_key_debug: String,
    /// If `private_key_debug` is set, then also is this field.
    pub public_key_debug: String,
}

impl PrivateManifest {
    const FILENAME_HIERARCHY: [&'static str; 1] = ["./.Samizdat.priv"];

    /// Find the private manifest, if one exists.
    pub fn find_opt() -> Result<Option<PrivateManifest>, anyhow::Error> {
        for filename in PrivateManifest::FILENAME_HIERARCHY {
            match fs::read(filename) {
                Ok(contents) => return Ok(Some(toml::from_slice(&contents)?)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }

        Ok(None)
    }

    /// Creates a new manifest and associated debug keypair, given debug series owner name and
    /// optionally production private key.
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
            private_key_debug: &PrivateKey::from(response.keypair.secret),
            public_key_debug: &Key::from(response.keypair.public),
        }
        .render()
        .expect("can render");

        fs::write("./.Samizdat.priv", rendered_private)?;
        let manifest = toml::from_str(&fs::read_to_string("./.Samizdat.priv")?)?;

        Ok(manifest)
    }
}
