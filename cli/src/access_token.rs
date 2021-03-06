use std::fs;

use crate::cli::cli;

static mut ACCESS_TOKEN: Option<String> = None;

/// Retrieves the access token. Must be called after initialization.
pub fn access_token<'a>() -> &'a str {
    unsafe { ACCESS_TOKEN.as_ref().expect("access token not initialized") }
}

/// Initializes access token. The access token is a file in the local
/// filesystem that grants access to protected routes in the Samizdat HTTP API.
pub fn init_access_token() -> Result<(), anyhow::Error> {
    let path = format!(
        "{}/access-token",
        cli().data.to_str().expect("path is not a string")
    );

    let access_token = fs::read_to_string(&path)?.trim().to_owned();

    // Set static:
    unsafe {
        ACCESS_TOKEN = Some(access_token);
    }

    Ok(())
}
