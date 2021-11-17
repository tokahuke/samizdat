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
