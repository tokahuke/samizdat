use std::path::PathBuf;
use structopt::StructOpt;

use crate::commands;

#[derive(Debug, StructOpt)]
pub enum Cli {
    /// Starts a new collection in this folder.
    Init,
    /// Creates a new version (collection) of the content in this folder.
    Commit {
        #[structopt(long)]
        ttl: Option<String>,
        dir: Option<PathBuf>,
    },
    /// Uploads a single file as an object.
    Upload {
        /// The content-type of this file. Will be guessed if unspecified.
        #[structopt(long)]
        content_type: Option<String>,
        #[structopt(long)]
        no_bookmark: bool,
        file: PathBuf,
    },
    /// Commands for managing series.
    Series {
        #[structopt(subcommand)]
        command: SeriesCommand,
    },
    /// Commands for managing collections.
    Collection {
        #[structopt(subcommand)]
        command: CollectionCommand,
    },
}

#[derive(Debug, StructOpt)]
pub enum SeriesCommand {
    /// Creates a new locally owned series.
    New { series_owner_name: String },
    /// Removes an existing locally owned series.
    Rm { series_owner_name: String },
    /// Imports a series from a `Samizdat.toml` in the current directory.
    Import {},
    /// Shows details on a particular locally owned series.
    Show { series_owner_name: String },
    /// Lists all loally owned series.
    Ls { series_owner_name: Option<String> },
}

#[derive(Debug, StructOpt)]
pub enum CollectionCommand {
    /// Shows details on a particular collection.
    Ls { collection: String },
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
            Cli::Commit { dir, ttl } => commands::commit(dir, ttl).await,
            Cli::Upload {
                file,
                content_type,
                no_bookmark,
            } => {
                let content_type = content_type.clone().unwrap_or_else(|| {
                    mime_guess::from_path(&file)
                        .first_or_octet_stream()
                        .to_string()
                });
                commands::upload(file, content_type, !no_bookmark).await
            }
            Cli::Series {
                command: SeriesCommand::New { series_owner_name },
            } => commands::series::new(series_owner_name.clone()).await,
            Cli::Series {
                command: SeriesCommand::Rm { series_owner_name },
            } => commands::series::rm(series_owner_name.clone()).await,
            Cli::Series {
                command: SeriesCommand::Import {},
            } => commands::series::import().await,
            Cli::Series {
                command: SeriesCommand::Show { series_owner_name },
            } => commands::series::show(series_owner_name.clone()).await,
            Cli::Series {
                command: SeriesCommand::Ls { series_owner_name },
            } => commands::series::list(series_owner_name).await,
            Cli::Collection {
                command: CollectionCommand::Ls { collection },
            } => commands::collection::ls(&collection).await,
        }
    }
}
