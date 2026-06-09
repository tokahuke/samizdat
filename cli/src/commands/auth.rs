//! Auth commands: grant and revoke rights to Web applications, and list
//! the current authorizations.

use tabled::Tabled;

use super::show_table;
use crate::api::{self, get_auths};

/// Grants `granted_rights` to `scope`. Fails silently with a NOTE if the
/// scope already has rights; revoke them first to replace.
pub async fn grant(scope: String, granted_rights: Vec<String>) -> Result<(), anyhow::Error> {
    let granted = api::patch_auth(&scope, api::PatchAuthRequest { granted_rights }).await?;

    if !granted {
        println!("NOTE: scope {scope} already has granted rights. Revoke them to grant new rights");
    }

    Ok(())
}

/// Revokes all rights from `scope`. Prints a NOTE if there were none.
pub async fn revoke(scope: String) -> Result<(), anyhow::Error> {
    let revoked = api::delete_auth(&scope).await?;

    if !revoked {
        println!("NOTE: scope {scope} had no granted rights");
    }

    Ok(())
}

/// Lists every scope with at least one granted right.
pub async fn ls() -> Result<(), anyhow::Error> {
    let auths = get_auths().await?;

    #[derive(Tabled)]
    struct Row {
        /// Authorization scope
        scope: String,
        /// List of granted rights
        granted_rights: String,
    }

    show_table(
        auths
            .into_iter()
            .map(|auth| Row {
                scope: if auth.entity.r#type == "_identity" {
                    format!("/{}", auth.entity.identifier)
                } else {
                    format!("/{}/{}", auth.entity.r#type, auth.entity.identifier)
                },
                granted_rights: auth.granted_rights.join(", "),
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}
