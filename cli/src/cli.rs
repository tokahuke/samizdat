use std::path::PathBuf;
use structopt::StructOpt;

use samizdat_common::{Hash, Key};

use crate::commands;

static mut CLI: Option<Cli> = None;

pub fn init_cli() -> Result<(), anyhow::Error> {
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
    format!("http://localhost:{}", crate::access_token::port())
}

#[derive(Clone, Debug, StructOpt)]
pub struct Cli {
    #[structopt(
        long,
        short,
        env = "SAMIZDAT_DATA",
        default_value = "/var/lib/samizdat/node"
    )]
    pub data: PathBuf,
    #[structopt(long, short = "v", env = "SAMIZDAT_VERBOSE")]
    pub verbose: bool,
    // DEPRECATED
    // #[structopt(long, short, env = "SAMIZDAT_PORT", default_value = "4510")]
    // pub port: u16,
    #[structopt(subcommand)]
    pub command: Command,
}

#[derive(Clone, Debug, StructOpt)]
pub enum Command {
    /// Starts a new collection in this folder.
    Init {
        #[structopt(long)]
        name: Option<String>,
    },
    /// Imports a series from a `Samizdat.toml` in the current directory.
    Import {
        /// The private key of the series.
        #[structopt(long)]
        private_key: Option<String>,
    },
    /// Creates a new version (collection) of the content in this folder.
    Commit {
        /// Set a custom time-to-leave for this commit.
        #[structopt(long)]
        ttl: Option<String>,
        /// Skip the build and go straight for the action!
        #[structopt(long)]
        skip_build: bool,
        /// Make this a release (for real) commit.
        #[structopt(long)]
        release: bool,
        /// Whether to announce this new edition to he network or to keep quiet.
        #[structopt(long)]
        no_announce: bool,
    },
    /// Watches the current directory for changes, rebuilding and committing at
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
        /// Don't bookmark this object. This makes is eligible for automatic deletion.
        #[structopt(long)]
        no_bookmark: bool,
        /// Sets this object as drafts. Drafts are not public to the network.
        #[structopt(long)]
        draft: bool,
        file: PathBuf,
    },
    // /// Downloads an object from the samizdat network.
    // Download {
    //
    // },
    /// Commands for managing series.
    Series {
        #[structopt(subcommand)]
        command: SeriesCommand,
    },
    /// Commands for managing editions.
    Edition {
        #[structopt(subcommand)]
        command: EditionCommand,
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
    /// Commands for managing identities.
    Identity {
        #[structopt(subcommand)]
        command: IdentityCommand,
    },
    /// Commands for managing authentication of scopes.
    Auth {
        #[structopt(subcommand)]
        command: AuthCommand,
    },
    /// Triggers a vacuum in the node. Vacuums remove junk from node storage and are
    /// run periodically, but you can trigger a manual run with this commmand.
    Vacuum {
        /// Use this flag to erase *all* OBJECT data in the node. Any data not backed up
        /// elsewhere WILL BE LOST. This command only affects objects; other entities,
        /// such as series and subscriptions will be unaffected.
        #[structopt(long)]
        flush_all: bool,
    },
}

impl Command {
    pub async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            Command::Init { name } => commands::init(name).await,
            Command::Import { private_key } => commands::import(private_key).await,
            Command::Commit {
                ttl,
                release,
                skip_build,
                no_announce,
            } => commands::commit(&ttl, skip_build, release, no_announce).await,
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
            Command::Edition { command } => command.execute().await,
            Command::Collection { command } => command.execute().await,
            Command::Subscription { command } => command.execute().await,
            Command::Identity { command } => command.execute().await,
            Command::Auth { command } => command.execute().await,
            Command::Vacuum { flush_all } => {
                if flush_all {
                    crate::api::post_flush_all().await
                } else {
                    crate::api::post_vacuum()
                        .await
                        .map(|status| println!("Vacuum status is: {status:?}"))
                }
            }
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum CollectionCommand {
    /// Shows details on a particular collection.
    Ls { collection: String },
}

impl CollectionCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            CollectionCommand::Ls { collection } => commands::collection::ls(collection).await,
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum SeriesCommand {
    /// Creates a new locally owned series.
    New {
        series_owner_name: String,
        #[structopt(long)]
        is_draft: bool,
        #[structopt(long)]
        public_key: Option<String>,
        #[structopt(long)]
        private_key: Option<String>,
    },
    /// Removes an existing locally owned series.
    Rm { series_owner_name: String },
    /// Shows details on a particular locally owned series.
    Show { series_owner_name: String },
    /// Lists all locally owned series.
    Ls { series_owner_name: Option<String> },
    /// Lists all known public keys the node has seen, be they locally owned or not.
    LsCached { series_name: Option<String> },
}

impl SeriesCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            SeriesCommand::New {
                series_owner_name,
                is_draft,
                public_key,
                private_key,
            } => commands::series::new(series_owner_name, is_draft, public_key, private_key).await,
            SeriesCommand::Rm { series_owner_name } => {
                commands::series::rm(series_owner_name).await
            }
            SeriesCommand::Show { series_owner_name } => {
                commands::series::show(series_owner_name).await
            }
            SeriesCommand::Ls { series_owner_name } => {
                commands::series::ls(series_owner_name).await
            }
            SeriesCommand::LsCached { series_name } => {
                commands::series::ls_cached(series_name).await
            }
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum EditionCommand {
    /// Lists all known editions or all known editions for a given series public key, if supplied.
    Ls { series_key: Option<String> },
}

impl EditionCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            EditionCommand::Ls { series_key } => commands::edition::ls(series_key).await,
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum SubscriptionCommand {
    /// Subscribe to a series. This tells the node to listen to announcements
    /// and to _actively_ keep in sync with the series.
    New { public_key: String },
    /// Trigger a manual refresh on this subscription, without waiting from a nudge from the network.
    Refresh { public_key: String },
    /// Removes an existing subscription.
    Rm { public_key: String },
    // /// Shows details on a particular subscription.
    // Show { public_key: String },
    /// Lists all subscriptions.
    Ls { public_key: Option<String> },
}

impl SubscriptionCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            SubscriptionCommand::New { public_key } => {
                commands::subscription::new(public_key).await
            }
            SubscriptionCommand::Refresh { public_key } => {
                commands::subscription::refresh(public_key).await
            }
            SubscriptionCommand::Rm { public_key } => commands::subscription::rm(public_key).await,
            // SubscriptionCommand::Show { public_key } => todo!(),
            SubscriptionCommand::Ls { public_key } => commands::subscription::ls(public_key).await,
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum IdentityCommand {
    /// Creates a new identity, putting the work to create a proof-of-work for it.
    Forge {
        /// Then handle (name) of the identity you want to forge.
        identity_handle: String,
        /// The name of the series owner for which you want to create an identity.
        series_owner_name: String,
        /// The number of iterations (per thread) to use to calculate proof-of-work.
        #[structopt(long)]
        n_iters: Option<usize>,
    },
    /// Imports an existing identity.
    Import {
        /// The handle (name) of the identity to be imported.
        identity_handle: String,
        /// The public key of the identity.
        #[structopt(long)]
        series: Key,
        /// The solution to the proof-of-work for this identity.
        #[structopt(long)]
        solution: Hash,
    },
    /// Lists all locally stored identities.
    Ls {
        /// An optional specific identity to be listed. If none is given, will list all existing
        /// identities.
        identity_handle: Option<String>,
    },
}

impl IdentityCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            IdentityCommand::Forge {
                identity_handle,
                series_owner_name,
                n_iters,
            } => commands::identity::forge(identity_handle, series_owner_name, n_iters).await,
            IdentityCommand::Ls { identity_handle } => {
                commands::identity::ls(identity_handle).await
            }
            IdentityCommand::Import {
                identity_handle,
                series,
                solution,
            } => commands::identity::import(identity_handle, series, solution).await,
        }
    }
}

#[derive(Clone, Debug, StructOpt)]
pub enum AuthCommand {
    Grant {
        scope: String,
        access_rights: Vec<String>,
    },
    Revoke {
        scope: String,
    },
}

impl AuthCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            AuthCommand::Grant {
                scope,
                access_rights,
            } => commands::auth::grant(scope, access_rights).await,
            AuthCommand::Revoke { scope } => commands::auth::revoke(scope).await,
        }
    }
}
