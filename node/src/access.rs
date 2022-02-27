use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};

use samizdat_common::Hash;

use crate::cli;

static mut ACCESS_TOKEN: Option<String> = None;

/// Retrieves the access token. Must be called after initialization.
pub fn access_token<'a>() -> &'a str {
    unsafe { ACCESS_TOKEN.as_ref().expect("access token not initialized") }
}

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

    let acess_token = match try_open_existing {
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
    unsafe {
        ACCESS_TOKEN = Some(acess_token);
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AccessRight {
    ManageObjects,
    GetObjectStats,
    ManageBookmarks,
    ManageCollections,
    ManageSeries,
    ManageSubscriptions,
    ManageIdentities,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    r#type: String,
    identifier: String,
}

impl Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}/{}", self.r#type, self.identifier)
    }
}

impl Entity {
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
