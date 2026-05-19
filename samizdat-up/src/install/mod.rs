//! Per-OS install/uninstall/update logic.
//!
//! Each platform module owns its service manager integration
//! (systemd, launchd, SCM). This module is the dispatcher: it picks
//! up the host platform via `cfg`, picks the right component path,
//! and routes `install` / `uninstall` / `update` to the platform
//! module.

use anyhow::Result;
use std::path::PathBuf;

use crate::cli::{AdminAction, Component};

/// Name of the system group that gates non-sudo access to the local
/// node's admin-token. Created at install time on Linux + macOS;
/// ignored on Windows.
pub const ADMIN_GROUP: &str = "samizdat";

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub struct InstallOpts {
    pub component: Component,
    pub version: Option<String>,
    pub no_service: bool,
    pub from: Option<String>,
    /// Unix user to run the daemons as. `None` keeps the
    /// service-manager default (root on Linux + macOS). The user
    /// must already exist on the host. See `cli.rs`'s
    /// `Command::Install::as_user` for the rationale.
    pub as_user: Option<String>,
}

pub struct UninstallOpts {
    pub component: Component,
    pub purge: bool,
}

pub fn install(opts: InstallOpts) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::install(opts);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::install(opts);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::install(opts);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = opts;
        anyhow::bail!("samizdat-up does not support this OS yet")
    }
}

pub fn uninstall(opts: UninstallOpts) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::uninstall(opts);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::uninstall(opts);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::uninstall(opts);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = opts;
        anyhow::bail!("samizdat-up does not support this OS yet")
    }
}

pub fn update(component: Option<Component>, to: Option<String>) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::update(component, to);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::update(component, to);
    }
    #[cfg(target_os = "windows")]
    {
        return windows::update(component, to);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (component, to);
        anyhow::bail!("samizdat-up does not support this OS yet")
    }
}

pub fn list() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::list();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::list();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::list();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        anyhow::bail!("samizdat-up does not support this OS yet")
    }
}

#[cfg(target_os = "windows")]
pub fn run_as_service(component: Component) -> Result<()> {
    windows::run_as_service(component)
}

pub fn admin(action: AdminAction) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::admin(action);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::admin(action);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = action;
        anyhow::bail!("`samizdat-up admin` is only supported on Linux and macOS")
    }
}

/// Paths of installed Samizdat binaries that exist on disk right now,
/// labelled by short name (`samizdat-node`, `samizdat`, `samizdat-up`,
/// ...). Used by `samizdat-up versions` to query each binary with
/// `--version`.
pub fn installed_binary_paths() -> Vec<(&'static str, PathBuf)> {
    #[cfg(target_os = "linux")]
    {
        return linux::installed_binary_paths();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::installed_binary_paths();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::installed_binary_paths();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Vec::new()
    }
}

pub fn self_update() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::self_update();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::self_update();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::self_update();
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        anyhow::bail!("samizdat-up does not support this OS yet")
    }
}
