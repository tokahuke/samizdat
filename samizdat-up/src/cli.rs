//! CLI surface for samizdat-up.

use clap::{Parser, Subcommand, ValueEnum};

use crate::install;

#[derive(Parser, Debug)]
#[command(name = "samizdat-up", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// The thing being installed/updated. Daemons (`node`, `hub`, `proxy`)
/// install as system services by default. `cli` is the user-facing
/// `samizdat` command-line tool and is also pulled in implicitly when
/// any daemon is installed (the daemon needs it for administration).
/// `all` is shorthand for `node`, `hub`, and `proxy` (cli rides
/// along).
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum Component {
    Node,
    Hub,
    Proxy,
    Cli,
    All,
}

impl Component {
    /// The daemons among this component selection. Empty for `cli`.
    pub fn daemons(self) -> Vec<&'static str> {
        match self {
            Component::Node => vec!["node"],
            Component::Hub => vec!["hub"],
            Component::Proxy => vec!["proxy"],
            Component::Cli => vec![],
            Component::All => vec!["node", "hub", "proxy"],
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Install a component + its CLI, register it as a system service,
    /// and start it.
    Install {
        /// What to install (default: node).
        #[arg(value_enum, default_value_t = Component::Node)]
        component: Component,
        /// Specific version to install. If omitted, the latest published
        /// edition of `get-samizdat` is used.
        #[arg(long)]
        version: Option<String>,
        /// Place the binary on disk but do NOT register or start the
        /// service. Useful for CI / packaging / debug.
        #[arg(long)]
        no_service: bool,
        /// Treat the given URL as the source of binaries instead of the
        /// public `get-samizdat` collection. Accepts http(s):// or
        /// file://. Used by the integration test workflow to install
        /// from locally-built artifacts.
        #[arg(long)]
        from: Option<String>,
        /// Run the daemon as this Unix user instead of the
        /// service-manager default (root on Linux + macOS). The user
        /// must already exist on the host; the data dir is chowned
        /// to them so the daemon can write its state and the
        /// admin-token is readable from that uid without sudo.
        /// Useful on a workstation install where the publisher is
        /// also the daemon's operator. Ignored on Windows (SCM
        /// account configuration uses a separate mechanism).
        #[arg(long, value_name = "USER")]
        as_user: Option<String>,
    },

    /// Stop the service, remove unit/plist/registration, remove the
    /// binary. Configs and data preserved unless `--purge` is set.
    Uninstall {
        /// What to remove (default: node).
        #[arg(value_enum, default_value_t = Component::Node)]
        component: Component,
        /// Also delete /etc/samizdat and /var/lib/samizdat (or the
        /// platform equivalents). Series private keys and the local
        /// object cache are gone after this.
        #[arg(long)]
        purge: bool,
    },

    /// Replace installed daemon(s) with a newer version, then restart.
    Update {
        /// Restrict to a specific component. If omitted, every installed
        /// daemon is updated.
        #[arg(value_enum)]
        component: Option<Component>,
        /// Pin a specific version (default: latest).
        #[arg(long)]
        to: Option<String>,
    },

    /// Print what is installed on this machine.
    List,

    /// Print available versions in the `get-samizdat` collection.
    Versions {
        /// Query the public collection instead of just listing locally
        /// known versions.
        #[arg(long)]
        remote: bool,
    },

    /// Replace samizdat-up itself with the latest published build.
    SelfUpdate,

    /// Manage who can administer the local node without `sudo`.
    ///
    /// samizdat-up creates a `samizdat` system group at install time
    /// and gives it group-read access to `admin-token`. Members of
    /// the group can run admin-scope CLI commands (publish, hub
    /// management, subscription mutations, etc.) without elevating
    /// to root. The group only exists on Linux + macOS.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },

    /// **Internal**: run as the SCM-managed service wrapper for one
    /// daemon. Not meant to be invoked by humans -- `samizdat-up
    /// install <component>` registers this subcommand as the
    /// `binPath` of a Windows service. SCM calls it when the service
    /// starts; this process then supervises the actual daemon binary.
    #[cfg(target_os = "windows")]
    #[command(hide = true)]
    Daemon {
        /// Which daemon to supervise. Must be node | hub | proxy.
        #[arg(value_enum)]
        component: Component,
    },
}

#[derive(Subcommand, Debug)]
pub enum AdminAction {
    /// Add `<user>` to the `samizdat` group. The user must log out
    /// and back in (or run `newgrp samizdat`) for membership to take
    /// effect in their shell.
    Add { user: String },
    /// Remove `<user>` from the `samizdat` group.
    Rm { user: String },
    /// List current members of the `samizdat` group.
    List,
}

impl Cli {
    pub fn run(self) -> Result<(), anyhow::Error> {
        match self.command {
            Command::Install {
                component,
                version,
                no_service,
                from,
                as_user,
            } => install::install(install::InstallOpts {
                component,
                version,
                no_service,
                from,
                as_user,
            }),
            Command::Uninstall { component, purge } => {
                install::uninstall(install::UninstallOpts { component, purge })
            }
            Command::Update { component, to } => install::update(component, to),
            Command::List => install::list(),
            Command::Versions { remote } => crate::fetch::list_versions(remote),
            Command::SelfUpdate => install::self_update(),
            Command::Admin { action } => install::admin(action),
            #[cfg(target_os = "windows")]
            Command::Daemon { component } => install::run_as_service(component),
        }
    }
}
