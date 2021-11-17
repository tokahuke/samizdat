use std::path::PathBuf;
use structopt::StructOpt;

use crate::commands;

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
    format!(
        "http://localhost:{}",
        cli()
            .port
    )
}

#[derive(Clone, Debug, StructOpt)]
pub struct Cli {
    #[structopt(long, short, env, default_value = "/var/samizdat/node")]
    pub data: PathBuf,
    #[structopt(long, short, env, default_value = "4510")]
    pub port: u16,
    #[structopt(subcommand)]
    pub command: Command,
}

#[derive(Clone, Debug, StructOpt)]
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
        /// Whether to announce this new edition to he network or to keep quiet.
        #[structopt(long)]
        no_annouce: bool,
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
    /// Commands for managing subscriptions.
    Subscription {
        #[structopt(subcommand)]
        command: SubscriptionCommand,
    },
}

impl Command {
    pub async fn execute(self) -> Result<(), crate::Error> {
        match self {
            Command::Init => commands::init().await,
            Command::Import => commands::import().await,
            Command::Commit {
                ttl,
                release,
                no_annouce,
            } => commands::commit(&ttl, release, no_annouce).await,
            Command::Watch { ttl } => commands::watch(&ttl).await,
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
                commands::upload(&file, content_type, !no_bookmark, draft).await
            }
            Command::Series { command } => command.execute().await,
            Command::Collection { command } => command.execute().await,
            Command::Subscription { command } => command.execute().await,
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum CollectionCommand {
    /// Shows details on a particular collection.
    Ls { collection: String },
}

impl CollectionCommand {
    async fn execute(self) -> Result<(), crate::Error> {
        match self {
            CollectionCommand::Ls { collection } => commands::collection::ls(collection).await,
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum SeriesCommand {
    /// Creates a new locally owned series.
    New { series_owner_name: String },
    /// Removes an existing locally owned series.
    Rm { series_owner_name: String },
    /// Shows details on a particular locally owned series.
    Show { series_owner_name: String },
    /// Lists all locally owned series.
    Ls { series_owner_name: Option<String> },
}

impl SeriesCommand {
    async fn execute(self) -> Result<(), crate::Error> {
        match self {
            SeriesCommand::New { series_owner_name } => {
                commands::series::new(series_owner_name).await
            }
            SeriesCommand::Rm { series_owner_name } => {
                commands::series::rm(series_owner_name).await
            }
            SeriesCommand::Show { series_owner_name } => {
                commands::series::show(series_owner_name).await
            }
            SeriesCommand::Ls { series_owner_name } => {
                commands::series::list(series_owner_name).await
            }
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum SubscriptionCommand {
    /// Subscribe to a series. This tells the node to listen to anouncements
    /// and to _actively_ keep in sync with the series.
    New { public_key: String },
    /// Removes an existing subscription.
    Rm { public_key: String },
    // /// Shows details on a particular subscription.
    // Show { public_key: String },
    /// Lists all subscriptions.
    Ls { public_key: Option<String> },
}

impl SubscriptionCommand {
    async fn execute(self) -> Result<(), crate::Error> {
        match self {
            SubscriptionCommand::New { public_key } => {
                commands::subscription::new(public_key).await
            }
            SubscriptionCommand::Rm { public_key } => commands::subscription::rm(public_key).await,
            // SubscriptionCommand::Show { public_key } => todo!(),
            SubscriptionCommand::Ls { public_key } => commands::subscription::ls(public_key).await,
        }
    }
}
