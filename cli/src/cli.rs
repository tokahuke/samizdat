//! Command-line interface module for the Samizdat application.
//!
//! This module provides the command-line argument parsing and execution logic for all
//! Samizdat commands.

use std::path::PathBuf;
use std::sync::OnceLock;
use structopt::StructOpt;

use crate::{api::EditionKind, commands};

/// Global CLI instance for the application
static CLI: OnceLock<Cli> = OnceLock::new();

/// Initializes the CLI by parsing command line arguments
pub fn init_cli() -> Cli {
    let cli = Cli::from_args();
    tracing::debug!("Arguments from command line: {:#?}", cli);
    cli
}

/// Returns a reference to the global CLI instance
pub fn cli<'a>() -> &'a Cli {
    CLI.get_or_init(init_cli)
}

/// Returns the server URL for the local Samizdat node
pub fn server() -> Result<String, anyhow::Error> {
    Ok(format!("http://localhost:{}", crate::access_token::port()?))
}

/// Main CLI configuration structure containing global options and subcommands
#[derive(Clone, Debug, StructOpt)]
pub struct Cli {
    /// Path to the Samizdat data directory
    #[structopt(
        long,
        short,
        env = "SAMIZDAT_DATA",
        default_value = "/var/lib/samizdat/node"
    )]
    pub data: PathBuf,

    /// Enable verbose logging
    #[structopt(long, short = "v", env = "SAMIZDAT_VERBOSE")]
    pub verbose: bool,

    /// The command to execute
    #[structopt(subcommand)]
    pub command: Command,
}

/// Available CLI commands for Samizdat
#[derive(Clone, Debug, StructOpt)]
pub enum Command {
    /// Tests if the server is up
    Up,

    /// Starts a new project in this folder
    Init {
        /// Optional name for the project
        #[structopt(long)]
        name: Option<String>,
    },

    /// Imports a series from a `Samizdat.toml` in the current directory
    Import {
        /// The private key of the series. If not provided, it will attempt to get the
        /// value from the privave manifest `.Samizdat.priv`
        ///
        #[structopt(long)]
        private_key: Option<String>,
    },

    /// Creates a new version (collection) of the content in this folder
    Commit {
        /// Set a custom time-to-leave for this commit
        #[structopt(long)]
        ttl: Option<String>,
        /// Skip the build and go straight for the action
        #[structopt(long)]
        skip_build: bool,
        /// Make this a release ("for realz") commit
        #[structopt(long)]
        release: bool,
        /// Whether to announce this new edition to the network
        #[structopt(long)]
        no_announce: bool,
        /// The kind of the commited edition.
        ///
        /// Either `base` (the default) or `layer`. Choosing base will override all th
        /// data while layer allows files not present in an edition to be found in
        /// previous editions.
        #[structopt(long, default_value = "base")]
        kind: EditionKind,
    },
    /// Watches the current directory for changes, rebuilding and committing at
    /// every change
    Watch {
        /// Set a custom time-to-leave for the commits
        #[structopt(long)]
        ttl: Option<String>,
        /// Suppresses opening web browser on first commit
        #[structopt(long)]
        no_browser: bool,
        /// The kind of the commited edition.
        ///
        /// Either `base` (the default) or `layer`. Choosing base will override all th
        /// data while layer allows files not present in an edition to be found in
        /// previous editions.
        #[structopt(long, default_value = "base")]
        kind: EditionKind,
    },
    /// Uploads a single file as an object. Use "-" to upload from stdin
    Upload {
        /// The content-type of this file. Will be guessed if unspecified
        #[structopt(long)]
        content_type: Option<String>,
        /// Don't bookmark this object. This makes is eligible for automatic deletion.
        #[structopt(long)]
        no_bookmark: bool,
        /// Sets this object as drafts. Drafts are not public to the network
        #[structopt(long)]
        draft: bool,
        // The file to upload. Alternatively, use "-" to upload from stdin.
        file: PathBuf,
    },
    /// Downloads an object from the samizdat network.
    Download {
        /// Hash of the object to download
        hash: String,
        /// Timeout for the resolution of the query in seconds
        #[structopt(long, default_value = "10")]
        timeout: u64,
    },
    /// Commands for managing hubs to which this node is connected.
    Hub {
        #[structopt(subcommand)]
        command: HubCommand,
    },
    /// Commands for managing hubs to which this node is connected.
    Connection {
        #[structopt(subcommand)]
        command: ConnectionCommand,
    },
    /// Commands for managing peers connected to this node.
    Peer {
        #[structopt(subcommand)]
        command: PeerCommand,
    },
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
    /// run periodically, but you can trigger a manual run with this command.
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
            Command::Up => {
                /* this is a no-op */
                Ok(())
            }
            Command::Init { name } => commands::init(name).await,
            Command::Import { private_key } => commands::import(private_key).await,
            Command::Commit {
                ttl,
                release,
                skip_build,
                no_announce,
                kind,
            } => commands::commit(&ttl, skip_build, release, no_announce, kind, None).await,
            Command::Watch {
                ttl,
                no_browser,
                kind,
            } => commands::watch(&ttl, no_browser, kind).await,
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
            Command::Download { hash, timeout } => commands::download(hash, timeout).await,
            Command::Hub { command } => command.execute().await,
            Command::Connection { command } => command.execute().await,
            Command::Peer { command } => command.execute().await,
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

/// Hub management commands for controlling hub connections.
/// 
/// These commands allow creating, listing, and removing hub connections that
/// the node uses to communicate with the Samizdat network.
#[derive(Clone, Debug, StructOpt)]
pub enum HubCommand {
    /// Creates a new hub connection
    New {
        /// Hub address
        address: String,
        /// Resolution mode
        resolution_mode: String,
    },
    /// Lists all hub connections
    Ls,
    /// Removes a hub connection
    Rm {
        /// Hub address to remove
        address: String,
    },
}

impl HubCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            HubCommand::New {
                address,
                resolution_mode,
            } => commands::hub::new(address, resolution_mode).await,
            HubCommand::Ls => commands::hub::ls().await,
            HubCommand::Rm { address } => commands::hub::rm(address).await,
        }
    }
}

/// Connection management commands for monitoring active hub connections.
/// 
/// These commands provide visibility into the current hub connections
/// maintained by the node.
#[derive(Clone, Debug, StructOpt)]
pub enum ConnectionCommand {
    /// Lists all active connections
    Ls,
}

impl ConnectionCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            ConnectionCommand::Ls => commands::connection::ls().await,
        }
    }
}

/// Peer management commands for interacting with network peers.
#[derive(Clone, Debug, StructOpt)]
pub enum PeerCommand {
    /// Lists all known peers
    Ls,
}

impl PeerCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            PeerCommand::Ls => commands::peer::ls().await,
        }
    }
}

/// Collection management commands for handling content collections.
/// 
/// These commands provide functionality to view and manage collections of
/// content within the Samizdat system.
#[derive(Clone, Debug, StructOpt)]
pub enum CollectionCommand {
    /// Shows details on a particular collection
    Ls {
        /// Collection identifier
        collection: String,
    },
}

impl CollectionCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            CollectionCommand::Ls { collection } => commands::collection::ls(collection).await,
        }
    }
}

/// Series management commands for handling content series.
/// 
/// These commands provide functionality to create, remove, and manage series,
/// which are sequences of related content editions in Samizdat.
#[derive(Clone, Debug, StructOpt)]
pub enum SeriesCommand {
    /// Creates a new locally owned series
    New {
        /// Name of the series owner
        series_owner_name: String,
        /// Whether the series is a draft
        #[structopt(long)]
        is_draft: bool,
        /// Optional public key
        #[structopt(long)]
        public_key: Option<String>,
        /// Optional private key
        #[structopt(long)]
        private_key: Option<String>,
    },
    /// Removes an existing locally owned series
    Rm {
        /// Name of the series owner
        series_owner_name: String,
    },
    /// Shows details on a particular locally owned series
    Show {
        /// Name of the series owner
        series_owner_name: String,
    },
    /// Lists all locally owned series
    Ls {
        /// Optional series owner name filter
        series_owner_name: Option<String>,
    },
    /// Lists all known public keys
    LsCached {
        /// Optional series name filter
        series_name: Option<String>,
    },
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

/// Edition management commands for handling content editions.
/// 
/// These commands allow viewing and managing editions, which are specific
/// versions of content within a series.
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

/// Subscription management commands for handling content subscriptions.
/// 
/// These commands enable users to subscribe to series, manage their subscriptions,
/// and control content synchronization from the network.
#[derive(Clone, Debug, StructOpt)]
pub enum SubscriptionCommand {
    /// Subscribe to a series
    New {
        /// Public key of the series
        public_key: String,
    },
    /// Trigger a manual refresh
    Refresh {
        /// Public key of the series
        public_key: String,
    },
    /// Removes an existing subscription, without waiting from a nudge from the network.
    Rm {
        /// Public key of the series
        public_key: String,
    },
    /// Lists all subscriptions
    Ls {
        /// Optional public key filter
        public_key: Option<String>,
    },
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

/// Identity management commands for blockchain-based identity operations.
/// 
/// These commands provide functionality to manage identities on the blockchain,
/// including creating and updating identity associations and managing blockchain
/// endpoints.
#[derive(Clone, Debug, StructOpt)]
pub enum IdentityCommand {
    /// Sets the Polygon blockchain provider endpoint
    SetEndpoint {
        /// Provider endpoint URL
        endpoint: String,
    },
    /// Gets the current Polygon blockchain provider endpoint
    GetEndpoint {},
    /// Creates a new association in the smart contact in the Polygon blockchain.
    Create {
        /// Human readable identity name
        identity: String,
        /// The **public** the identity will refer to.
        entity: String,
        /// The time-to-leave in seconds (time in cache) of this rule.
        #[structopt(long, default_value = "3600")]
        ttl: u64,
        /// Optional custom blockchain endpoint
        #[structopt(long)]
        endpoint: Option<String>,
    },
    /// Updates an existing blockchain identity association
    Update {
        /// Human readable identity name
        identity: String,
        /// New public key to associate
        entity: String,
        /// Cache time-to-live in seconds
        #[structopt(long, default_value = "3600")]
        ttl: u64,
        /// Optional custom blockchain endpoint
        #[structopt(long)]
        endpoint: Option<String>,
    },
    /// Gets the current key for an identity
    Get {
        /// Identity to resolve
        identity: String,
        /// Optional custom blockchain endpoint
        #[structopt(long)]
        endpoint: Option<String>,
    },
}

impl IdentityCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            IdentityCommand::SetEndpoint { endpoint } => {
                commands::identity::set_provider(&endpoint).await
            }
            IdentityCommand::GetEndpoint {} => commands::identity::get_provider().await,
            IdentityCommand::Create {
                identity,
                entity,
                ttl,
                endpoint,
            } => commands::identity::create(identity, entity, ttl, endpoint).await,
            IdentityCommand::Update {
                identity,
                entity,
                ttl,
                endpoint,
            } => commands::identity::update(identity, entity, ttl, endpoint).await,
            IdentityCommand::Get { identity, endpoint } => {
                commands::identity::get(identity, endpoint).await
            }
        }
    }
}

/// Authentication management commands for controlling access rights.
/// 
/// These commands allow managing access control through granting and revoking
/// rights to different Web application scopes.
#[derive(Clone, Debug, StructOpt)]
pub enum AuthCommand {
    /// Grants access rights to a scope
    Grant {
        /// Scope identifier
        scope: String,
        /// Comma-separated list of rights to grant.
        access_rights: Vec<String>,
    },
    /// Revokes all rights from a scope
    Revoke {
        /// Scope identifier
        scope: String,
    },
    /// Lists all current rights granted to an entity
    Ls {},
}

impl AuthCommand {
    async fn execute(self) -> Result<(), anyhow::Error> {
        match self {
            AuthCommand::Grant {
                scope,
                access_rights,
            } => commands::auth::grant(scope, access_rights).await,
            AuthCommand::Revoke { scope } => commands::auth::revoke(scope).await,
            AuthCommand::Ls {} => commands::auth::ls().await,
        }
    }
}
