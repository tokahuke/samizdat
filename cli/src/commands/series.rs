//! Series command implementations for the Samizdat CLI.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use anyhow::Context;
use tabled::Tabled;

use samizdat_common::{Key, PrivateKey};

use super::show_table;
use crate::api::{self, Keypair};

/// Creates a new series with the specified parameters.
///
/// The private key, when supplied, is read from a FILE path rather than from
/// `argv`. This keeps the secret out of `ps`, shell history, and audit logs.
///
/// # Arguments
///
/// * `series_name` - Name of the series to create
/// * `is_draft` - Whether this is a draft series. Draft series are not published
///   to the network.
/// * `public_key` - Optional public key for the series
/// * `private_key_file` - Optional path to a file containing the private key
pub async fn new(
    series_name: String,
    is_draft: bool,
    public_key: Option<String>,
    private_key_file: Option<PathBuf>,
) -> Result<(), anyhow::Error> {
    if public_key.is_some() && private_key_file.is_none() {
        anyhow::bail!("Missing private key (pass --private-key-file <PATH>)")
    } else if public_key.is_none() && private_key_file.is_some() {
        anyhow::bail!("Missing public key (pass --public-key <KEY>)")
    }

    let private_key = private_key_file
        .map(|path| -> anyhow::Result<String> {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("reading private key from {}", path.display()))?;
            // Validate format before sending; `parse::<PrivateKey>()` rejects
            // malformed input but the wire payload still uses the trimmed
            // string representation.
            let trimmed = raw.trim().to_owned();
            let _: samizdat_common::PrivateKey = trimmed.parse()?;
            Ok(trimmed)
        })
        .transpose()?;

    let keypair = public_key
        .zip(private_key)
        .map(|(public_key, private_key)| Keypair {
            public_key,
            private_key,
        });

    api::post_series_owner(api::PostSeriesOwnerRequest {
        series_owner_name: &series_name,
        keypair,
        is_draft,
    })
    .await?;

    Ok(())
}

/// Removes a series owner. This is destructive: it deletes the series's
/// private key on the node, after which only nodes that already cached the
/// public key can resolve the series, and only with whatever editions they
/// already have. Prompts for confirmation unless `assume_yes` is true.
///
/// # Arguments
/// * `series_name` - Name of the series to remove
/// * `assume_yes` - Skip the interactive prompt (e.g. for non-interactive scripts)
pub async fn rm(series_name: String, assume_yes: bool) -> Result<(), anyhow::Error> {
    if !assume_yes {
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "Refusing to remove series {series_name}: stdin is not a TTY and \
                 --yes was not supplied. This is a destructive operation."
            );
        }
        print!(
            "Permanently remove series owner {series_name}? \
             This destroys the series private key on the node. [y/N] "
        );
        std::io::stdout().flush().ok();
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
            println!("Aborted.");
            return Ok(());
        }
    }

    let removed = api::delete_series_owner(&series_name).await?;

    if !removed {
        println!("NOTE: series {series_name} does not exist.");
    }

    Ok(())
}

/// Shows information about a specific series.
///
/// # Arguments
/// * `series_name` - Name of the series to show
pub async fn show(series_name: String) -> Result<(), anyhow::Error> {
    api::get_series_owner(&series_name).await?;
    Ok(())
}

/// Lists series owners, either all or for a specific series.
///
/// # Arguments
/// * `series_owner_name` - Optional name of the series owner to list
pub async fn ls(series_owner_name: Option<String>) -> Result<(), anyhow::Error> {
    #[derive(Tabled)]
    struct Row {
        /// Name of the series owner
        name: String,
        /// Public key of the series
        public_key: Key,
        /// Private key of the series
        private_key: PrivateKey,
        /// Default time-to-live duration
        default_ttl: String,
    }

    async fn ls_series(series_owner_name: String) -> Result<(), anyhow::Error> {
        let series_owner = api::get_series_owner(&series_owner_name).await?;

        show_table(vec![Row {
            name: series_owner.name,
            public_key: series_owner.keypair.verifying_key().into(),
            private_key: series_owner.keypair.to_scalar_bytes().into(),
            default_ttl: format!("{:?}", series_owner.default_ttl),
        }]);

        Ok(())
    }

    async fn ls_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_series_owners().await?;

        show_table(
            response
                .into_iter()
                .map(|series_owner| Row {
                    name: series_owner.name,
                    public_key: series_owner.keypair.verifying_key().into(),
                    private_key: series_owner.keypair.to_scalar_bytes().into(),
                    default_ttl: format!("{:?}", series_owner.default_ttl),
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    if let Some(series_owner_name) = series_owner_name {
        ls_series(series_owner_name).await
    } else {
        ls_all().await
    }
}

/// Lists cached series information, either all or for a specific series.
///
/// # Arguments
/// * `series_name` - Optional name of the series to list cached information for
pub async fn ls_cached() -> Result<(), anyhow::Error> {
    async fn ls_cached_all() -> Result<(), anyhow::Error> {
        let response = api::get_all_series().await?;

        #[derive(Tabled)]
        struct Row {
            /// Public key of the series
            public_key: Key,
        }

        show_table(
            response
                .into_iter()
                .map(|series| Row {
                    public_key: series.public_key,
                })
                .collect::<Vec<_>>(),
        );

        Ok(())
    }

    ls_cached_all().await
}
