//! The `Samizdat.toml` manifest format.
//! 

use serde_derive::{Deserialize};
use std::{fs, io, env};
use std::path::PathBuf;
use std::process::Command;

#[derive(askama::Template)]
#[template(path = "Samizdat.toml.txt")]
pub struct ManifestTemplate<'a> {
    pub name: &'a str,
    pub public_key: &'a str,
    pub ttl: &'a str,
}

#[derive(Deserialize)]
pub struct Manifest {
    pub series: Series,
    pub build: Build,
}

impl Manifest {
    pub fn find() -> Result<Manifest, crate::Error> {
        let filename_hierarchy = ["./Samizdat.toml", "./Samizdat.tml", "./samizdat.toml", "./samizdat.tml"];
        let mut last_error = None;

        for filename in filename_hierarchy {
            match fs::read(filename) {
                Ok(contents) => return Ok(
                    toml::from_slice(&contents)?
                ),
                Err(err) if err.kind() == io::ErrorKind::NotFound => last_error = Some(err),
                Err(err) => return Err(err.into()),
            }
        }

        Err(last_error.expect("filename hierarchy not empty").into())
    }
}

#[derive(Deserialize)]
pub struct Series {
    pub name: String,
    pub ttl: Option<String>,
}

#[derive(Deserialize)]
pub struct Build {
    pub base: PathBuf,
    pub run: Option<String>,
}

impl Build {
    pub fn run(&self) -> Result<(), crate::Error> {
        let mut command = Command::new(env::var("SHELL").expect("shell exists"));
        command
            .arg("-c")
            .arg(self.run.clone().unwrap_or_default());

        println!("Running {:?}", command);

        let status = command
            .spawn()?
            .wait()?;

        if status.success() {
            Ok(())
        } else {
            Err(crate::Error::Message(format!("bad exit status for run command: {}", status)))
        }
    }
}
