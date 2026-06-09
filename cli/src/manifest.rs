//! The `Samizdat.toml` manifest format.
//!
//! Handles both the public manifest (`Samizdat.toml`, project metadata
//! and build settings) and the private one (`.Samizdat.priv`, series
//! keypairs). The private one is secrets-only and must be in
//! `.gitignore`.

use askama::Template;
use serde_derive::Deserialize;
use std::{fs, io, path::PathBuf, process::Command};

use samizdat_common::{Key, PrivateKey};

use crate::api;

/// Template for new `Samizdat.toml` files.
#[derive(askama::Template)]
#[template(path = "Samizdat.toml.txt")]
pub struct ManifestTemplate<'a> {
    /// Node-local nickname for the series owner.
    pub nickname: &'a str,
    /// Public key for the series.
    pub public_key: &'a Key,
    /// TTL for series content, as a human-readable duration string.
    pub ttl: &'a str,
}

/// A loaded Samizdat project manifest.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    /// Series configuration.
    pub series: Series,
    /// Debug-environment overrides.
    pub debug: Debug,
    /// How to build this project.
    pub build: Build,
}

impl Manifest {
    /// Manifest filenames the loader tries, in order of preference.
    const FILENAME_HIERARCHY: [&'static str; 4] = [
        "./Samizdat.toml",
        "./Samizdat.tml",
        "./samizdat.toml",
        "./samizdat.tml",
    ];

    /// Load the manifest from the current directory. Returns `None` if
    /// none of the candidate filenames exist.
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

    /// Create a new manifest plus the associated debug keypair. Bails if
    /// a manifest already exists in the current directory.
    pub async fn create(nickname: &str) -> Result<(Manifest, PrivateKey), anyhow::Error> {
        if Manifest::find_opt()?.is_some() {
            anyhow::bail!("`Samizdat.toml` already exists.");
        }

        // The nickname flows raw into a TOML template (Askama's `.txt` extension
        // uses the no-op `Text` escaper). Reject anything that would break the
        // resulting file or smuggle additional keys. Allowed: letters, digits,
        // `-`, `_`, `.`, `/`, space. Names without this restriction could
        // contain `"` to terminate the string, or newlines to inject sections.
        validate_nickname(nickname)?;

        let response = api::post_series_owner(api::PostSeriesOwnerRequest {
            nickname,
            keypair: None,
            is_draft: false,
        })
        .await?;

        let rendered = crate::manifest::ManifestTemplate {
            nickname,
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

    /// Run the build defined by the manifest. `is_release` switches
    /// between the release and debug build scripts.
    pub fn run_build(&self, is_release: bool) -> Result<(), anyhow::Error> {
        self.build.run(&self.series.public_key, is_release)
    }
}

/// `[series]` section of the manifest.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Series {
    /// Node-local nickname for this series owner.
    pub nickname: String,
    /// Public key for the series.
    pub public_key: String,
    /// Optional TTL for series content (human-readable duration).
    pub ttl: Option<String>,
}

/// `[debug]` section of the manifest.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Debug {
    /// Node-local nickname for the debug series owner.
    pub nickname: String,
}

/// Default shell: `$SHELL`, or `/bin/sh` if not set.
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
}

/// `[build]` section of the manifest.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Build {
    /// Directory Samizdat reads the produced content from when creating
    /// a new edition of the series.
    pub base: PathBuf,
    /// Command for release builds.
    pub run: Option<String>,
    /// Command for debug builds.
    pub run_debug: Option<String>,
    /// Shell used to run the build command.
    #[serde(default = "default_shell")]
    pub shell: String,
}

impl Build {
    /// Run the build script and wait for it to finish.
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

/// Template for new `.Samizdat.priv` files.
#[derive(askama::Template)]
#[template(path = "Samizdat.priv.txt")]
pub struct PrivateManifestTemplate<'a> {
    /// Optional production private key.
    pub private_key: Option<&'a PrivateKey>,
    /// Debug-environment private key.
    pub private_key_debug: &'a PrivateKey,
    /// Debug-environment public key.
    pub public_key_debug: &'a Key,
}

/// The secrets-only side of a Samizdat project manifest.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PrivateManifest {
    /// Optional production private key.
    pub private_key: Option<String>,
    /// Debug-environment private key.
    pub private_key_debug: String,
    /// Debug-environment public key.
    pub public_key_debug: String,
}

impl PrivateManifest {
    /// Private-manifest filenames the loader tries.
    const FILENAME_HIERARCHY: [&'static str; 1] = ["./.Samizdat.priv"];

    /// Load the private manifest from the current directory. Returns
    /// `None` if no candidate filename exists.
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

    /// Create a new private manifest. The debug keypair is allocated by
    /// the node; `private_key` is the optional production key (created
    /// elsewhere). Bails if `.Samizdat.priv` already exists.
    pub async fn create(
        debug_nickname: &str,
        private_key: Option<&PrivateKey>,
    ) -> Result<PrivateManifest, anyhow::Error> {
        if PrivateManifest::find_opt()?.is_some() {
            anyhow::bail!("`.Samizdat.priv` already exists.");
        }

        let response = api::post_series_owner(api::PostSeriesOwnerRequest {
            nickname: debug_nickname,
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

        write_priv_file("./.Samizdat.priv", rendered_private.as_bytes())?;
        let manifest = toml::from_str(&fs::read_to_string("./.Samizdat.priv")?)?;

        Ok(manifest)
    }
}

/// Validate a series-owner nickname before it gets rendered into
/// `Samizdat.toml`.
///
/// The Askama template uses the no-op `Text` escaper because the output
/// is TOML, not HTML; the nickname is dropped raw inside
/// `nickname = "{{ nickname }}"`. Letting `"`, `\`, or newlines through
/// would produce a malformed file or let a careless directory name
/// (or an attacker) inject extra TOML keys. Allowed: the same alphabet
/// paths and URLs use, plus a few punctuation marks. Anything else is
/// rejected.
fn validate_nickname(nickname: &str) -> Result<(), anyhow::Error> {
    if nickname.is_empty() {
        anyhow::bail!("nickname must not be empty");
    }
    if nickname.len() > 128 {
        anyhow::bail!("nickname is too long (max 128 chars)");
    }
    for c in nickname.chars() {
        let ok = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ' ');
        if !ok {
            anyhow::bail!(
                "nickname contains disallowed character {c:?}; \
                 use letters, digits, '-', '_', '.', '/' or space"
            );
        }
    }
    Ok(())
}

/// Write a file containing secret material. On Unix the file is created
/// mode 0o600 (owner read/write only). Elsewhere, `create_new` keeps us
/// from clobbering an existing file by accident.
fn write_priv_file(path: &str, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(contents)?;
    Ok(())
}
