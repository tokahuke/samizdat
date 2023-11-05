//! Access rights infrastructure for the node.

use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};

use samizdat_common::Hash;

use crate::cli;

/// The access token is a file in the local filesystem that grants access to protected
/// routes in the Samizdat HTTP API. This avoids unauthorized access from scripts running
/// in webpages.
static mut ACCESS_TOKEN: Option<String> = None;

/// Retrieves the access token. Must be called after initialization.
pub fn access_token<'a>() -> &'a str {
    unsafe { ACCESS_TOKEN.as_ref().expect("access token not initialized") }
}

/// Generates a new access token value.
fn gen_token() -> String {
    Hash::rand().to_string()
}

/// Initializes access token. The access token is a file in the local
/// filesystem that grants access to protected routes in the Samizdat HTTP API.
pub fn init_access_token() -> Result<(), crate::Error> {
    let path = format!(
        "{}/access-token",
        cli().data.to_str().expect("path is not a string")
    );
    let try_open_existing = OpenOptions::new().write(true).create_new(true).open(&path);

    let access_token = match try_open_existing {
        Ok(mut file) => {
            let access_token = gen_token();
            file.write_all(access_token.as_bytes())?;
            access_token
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::read_to_string(&path)?.trim().to_owned()
        }
        Err(error) => return Err(error.into()),
    };

    // Set static:
    log::info!("Node access token is {access_token:?}");
    unsafe {
        ACCESS_TOKEN = Some(access_token);
    }

    // ... and also piggyback writing port here. I know this is hacky, but...
    let port_path = format!(
        "{}/port",
        cli().data.to_str().expect("path is not a string")
    );
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(port_path)?;
    file.write(cli().port.to_string().as_bytes())?;

    Ok(())
}

/// Access rights that can be granted to web applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AccessRight {
    /// Can create and delete objects.
    ManageObjects,
    /// Can get statistics on object usage.
    GetObjectStats,
    /// Can create and delete bookmarks.
    ManageBookmarks,
    /// Can create collections.
    ManageCollections,
    /// Can create and delete series (including private keys).
    ManageSeries,
    /// Can create and delete subscriptions.
    ManageSubscriptions,
    /// Can create and delete identities.
    ManageIdentities,
    /// Can create and delete connection to Samizdat Hubs.
    ManageHubs,
}

/// A name of an entity inside the Samizdat network. An entity can be an object, a
/// collection item, a series item, etc...
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    /// The type of the entity.
    r#type: String,
    /// The identifier of the entity.
    identifier: String,
}

impl Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}/{}", self.r#type, self.identifier)
    }
}

impl Entity {
    /// Transforms an URL path into an entity, if possible.
    pub fn from_path(path: &str) -> Option<Entity> {
        let mut split = path.split('/');
        let mut r#type = split.next()?;

        if r#type.is_empty() {
            r#type = split.next()?;
        }

        let identifier = split.next()?;

        Some(Entity {
            r#type: r#type.to_owned(),
            identifier: identifier.to_owned(),
        })
    }
}
