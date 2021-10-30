use std::path::PathBuf;
use structopt::StructOpt;

use crate::commands;

#[derive(Debug, StructOpt)]
pub struct Cli {
    #[structopt(long, short, env, default_value = "4510")]
    pub port: u16,
    #[structopt(subcommand)]
    pub command: Command,
}

#[derive(Debug, StructOpt)]
pub enum Command {
    /// Starts a new collection in this folder.
    Init,
    /// Imports a series from a `Samizdat.toml` in the current directory.
    Import,
    /// Creates a new version (collection) of the content in this folder.
    Commit {
        /// Set a custom time-to-leave for this commit.
        #[structopt(long)]
        ttl: Option<String>,
        /// Make this a release (for real) commit.
        #[structopt(long)]
        release: bool,
    },
    /// Watches the current directory for changes, rebilding and commiting at
    /// every change.
    Watch {
        /// Set a custom time-to-leave for the commits.
        #[structopt(long)]
        ttl: Option<String>,
    },
    /// Uploads a single file as an object.
    Upload {
        /// The content-type of this file. Will be guessed if unspecified.
        #[structopt(long)]
        content_type: Option<String>,
        /// Don't bookmark this object. This makes is ellegible for automatic deletion.
        #[structopt(long)]
        no_bookmark: bool,
        /// Sets this object as drafts. Drafts are not public to the network.
        #[structopt(long)]
        draft: bool,
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

pub fn server() -> String {
    format!("http://localhost:{}", cli().port)
}

impl Command {
    pub async fn execute(&self) -> Result<(), crate::Error> {
        match self {
            Command::Init => commands::init().await,
            Command::Import => commands::import().await,
            Command::Commit { ttl, release } => commands::commit(ttl, *release).await,
            Command::Watch { ttl } => commands::watch(ttl).await,
            Command::Upload {
                file,
                content_type,
                no_bookmark,
                draft,
            } => {
                let content_type = content_type.clone().unwrap_or_else(|| {
                    mime_guess::from_path(&file)
                        .first_or_octet_stream()
                        .to_string()
                });
                commands::upload(file, content_type, !no_bookmark, *draft).await
            }
            Command::Series {
                command: SeriesCommand::New { series_owner_name },
            } => commands::series::new(series_owner_name.clone()).await,
            Command::Series {
                command: SeriesCommand::Rm { series_owner_name },
            } => commands::series::rm(series_owner_name.clone()).await,
            Command::Series {
                command: SeriesCommand::Show { series_owner_name },
            } => commands::series::show(series_owner_name.clone()).await,
            Command::Series {
                command: SeriesCommand::Ls { series_owner_name },
            } => commands::series::list(series_owner_name).await,
            Command::Collection {
                command: CollectionCommand::Ls { collection },
            } => commands::collection::ls(collection).await,
        }
    }
}
