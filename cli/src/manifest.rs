//! The `Samizdat.toml` manifest format.
//!

use serde_derive::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::{fs, io};

#[derive(askama::Template)]
#[template(path = "Samizdat.toml.txt")]
pub struct ManifestTemplate<'a> {
    pub name: &'a str,
    pub public_key: &'a str,
    pub ttl: &'a str,
    pub debug_name: &'a str,
    pub public_key_debug: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    pub series: Series,
    pub debug: Series,
    pub build: Build,
}

impl Manifest {
    pub fn find() -> Result<Manifest, crate::Error> {
        let filename_hierarchy = [
            "./Samizdat.toml",
            "./Samizdat.tml",
            "./samizdat.toml",
            "./samizdat.tml",
        ];
        let mut last_error = None;

        for filename in filename_hierarchy {
            match fs::read(filename) {
                Ok(contents) => return Ok(toml::from_slice(&contents)?),
                Err(err) if err.kind() == io::ErrorKind::NotFound => last_error = Some(err),
                Err(err) => return Err(err.into()),
            }
        }

        Err(last_error.expect("filename hierarchy not empty").into())
    }

    pub fn run(&self, is_release: bool) -> Result<(), crate::Error> {
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

fn default_shell() -> String {
    "/usr/bin/bash".to_owned()
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
    pub fn run(&self, public_key: &str, is_release: bool) -> Result<(), crate::Error> {
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
            Err(crate::Error::Message(format!(
                "bad exit status for run command: {}",
                status
            )))
        }
    }
}

#[derive(askama::Template)]
#[template(path = "Samizdat.priv.txt")]
pub struct PrivateManifestTemplate<'a> {
    pub private_key: &'a str,
    pub private_key_debug: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PrivateManifest {
    pub private_key: String,
    pub private_key_debug: String,
}

impl PrivateManifest {
    pub fn find() -> Result<PrivateManifest, crate::Error> {
        let filename_hierarchy = ["./.Samizdat.priv"];
        let mut last_error = None;

        for filename in filename_hierarchy {
            match fs::read(filename) {
                Ok(contents) => return Ok(toml::from_slice(&contents)?),
                Err(err) if err.kind() == io::ErrorKind::NotFound => last_error = Some(err),
                Err(err) => return Err(err.into()),
            }
        }

        Err(last_error.expect("filename hierarchy not empty").into())
    }
}
