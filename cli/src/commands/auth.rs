//! Authentication command implementations for the Samizdat CLI.
//!
//! This module provides functionality for managing authentication and authorization,
//! including granting and revoking rights to Web applications, and listing current
//! authorizations.

use tabled::Tabled;

use super::show_table;
use crate::api::{self, get_auths};

/// Grants rights to a specific scope.
///
/// # Arguments
/// * `scope` - The target scope for granting rights
/// * `granted_rights` - List of rights to be granted
pub async fn grant(scope: String, granted_rights: Vec<String>) -> Result<(), anyhow::Error> {
    let granted = api::patch_auth(&scope, api::PatchAuthRequest { granted_rights }).await?;

    if !granted {
        println!("NOTE: scope {scope} already has granted rights. Revoke them to grant new rights");
    }

    Ok(())
}

/// Revokes all rights from a specific scope.
///
/// # Arguments
/// * `scope` - The target scope from which to revoke rights
pub async fn revoke(scope: String) -> Result<(), anyhow::Error> {
    let revoked = api::delete_auth(&scope).await?;

    if !revoked {
        println!("NOTE: scope {scope} had no granted rights");
    }

    Ok(())
}

/// Lists all current authorization scopes and their granted rights.
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
