use std::path::PathBuf;
use structopt::StructOpt;

use crate::commands;

#[derive(Debug, StructOpt)]
pub enum Cli {
    /// Starts a new collection in this folder.
    Init,
    /// Creates a new version (collection) of the content in this folder.
    Commit { dir: PathBuf },
    /// Uploads a single file as an object.
    Upload {
        /// The content-type of this file. Will be guessed if unspecified.
        #[structopt(long)]
        content_type: Option<String>,
        file: PathBuf,
    },
}

static mut CLI: Option<Cli> = None;

pub fn init_cli() -> Result<(), crate::Error> {
    let cli = Cli::from_args();

    log::debug!("Arguments from command line: {:#?}", cli);

    unsafe {
        CLI = Some(cli);
    }

    Ok(())
}

pub fn cli<'a>() -> &'a Cli {
    unsafe { CLI.as_ref().expect("cli not initialized") }
}

impl Cli {
    pub async fn execute(&self) -> Result<(), crate::Error> {
        match self {
            Cli::Init => commands::init().await,
            Cli::Commit { dir } => commands::commit(dir).await,
            Cli::Upload { file, content_type } => {
                let content_type = content_type.clone().unwrap_or_else(|| {
                    mime_guess::from_path(&file)
                        .first_or_octet_stream()
                        .to_string()
                });
                commands::upload(file, content_type).await
            }
        }
    }
}
