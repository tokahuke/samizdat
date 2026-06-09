//! Per-OS install/uninstall/update logic.
//!
//! Each platform module owns its service manager integration
//! (systemd, launchd, SCM). This module is the dispatcher: it picks
//! up the host platform via `cfg`, picks the right component path,
//! and routes `install` / `uninstall` / `update` to the platform
//! module.

use anyhow::Result;
use std::{
    net::TcpStream,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use crate::cli::{AdminAction, Component};

/// Name of the system group that gates non-sudo access to the local
/// node's admin-token. Created at install time on Linux + macOS;
/// ignored on Windows.
pub const ADMIN_GROUP: &str = "samizdat";

/// Default hubs registered on a fresh `samizdat-up install node`. Kept
/// here (post-install, not in the node daemon) because seeding belongs
/// to the install lifecycle, not every node start. The list is
/// hardcoded; treat as necessary tech debt until there is a better
/// place for it. Operators who do not want these can delete them with
/// `samizdat hub rm <address>`.
const DEFAULT_HUBS: &[(&str, &str)] = &[("testbed.hubfederation.com", "use-both")];

/// TCP-ping the node's HTTP port until it accepts a connection or the
/// timeout elapses. Used after enabling the service to know whether
/// `samizdat hub new ...` can be safely invoked.
fn wait_for_node_up(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let port = node_http_port();
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().expect("valid socket"),
            Duration::from_millis(500),
        )
        .is_ok()
        {
            return true;
        }
        thread::sleep(Duration::from_millis(250));
    }
    false
}

/// Default samizdat-node HTTP port. Mirrors the
/// `default_value = "4510"` in `node/src/cli.rs`. If we ever support a
/// non-default port at install time, this should pick it up from the
/// effective config rather than hardcoding.
fn node_http_port() -> u16 {
    4510
}

/// Best-effort post-install seeding: register the hardcoded
/// [`DEFAULT_HUBS`] on the just-installed node. Idempotent because the
/// node's `Hubs::insert` dedupes by address; re-installs over an
/// existing data dir are harmless. On failure (node didn't come up in
/// time, `samizdat` CLI not on PATH, etc.) we print one warning and
/// return Ok so the install does not fail.
fn seed_default_hubs_best_effort() {
    if !wait_for_node_up(Duration::from_secs(15)) {
        eprintln!(
            "samizdat-up: node did not accept TCP connections within 15s; \
             skipping default-hub seeding. Run `samizdat hub new \
             <address> <resolution-mode>` once it is up."
        );
        return;
    }

    let samizdat = installed_binary_paths()
        .into_iter()
        .find(|(name, _)| *name == "samizdat")
        .map(|(_, path)| path);

    let Some(samizdat) = samizdat else {
        eprintln!("samizdat-up: cannot locate `samizdat` binary; skipping default-hub seeding.");
        return;
    };

    for (address, resolution_mode) in DEFAULT_HUBS {
        match Command::new(&samizdat)
            .args(["hub", "new", address, resolution_mode])
            .status()
        {
            Ok(status) if status.success() => {
                println!("samizdat-up: seeded default hub {address} ({resolution_mode})");
            }
            Ok(status) => {
                eprintln!(
                    "samizdat-up: `samizdat hub new {address} {resolution_mode}` exited with \
                     {status}. Re-run manually if you want this hub configured."
                );
            }
            Err(err) => {
                eprintln!(
                    "samizdat-up: could not spawn `samizdat hub new`: {err}. Re-run manually \
                     if you want this hub configured."
                );
            }
        }
    }
}

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
    // Capture before move; seeding only matters when the node daemon
    // was just enabled.
    let installs_node = matches!(opts.component, Component::Node | Component::All);
    let no_service = opts.no_service;

    #[cfg(target_os = "linux")]
    let result = linux::install(opts);
    #[cfg(target_os = "macos")]
    let result = macos::install(opts);
    #[cfg(target_os = "windows")]
    let result = windows::install(opts);
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let result: Result<()> = {
        let _ = opts;
        anyhow::bail!("samizdat-up does not support this OS yet")
    };

    result?;

    if installs_node && !no_service {
        seed_default_hubs_best_effort();
    }

    Ok(())
}

pub fn uninstall(opts: UninstallOpts) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        return linux::uninstall(opts);
    }
    #[cfg(target_os = "macos")]
    {
        macos::uninstall(opts)
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
        macos::update(component, to)
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
        macos::list()
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
        macos::admin(action)
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
        macos::installed_binary_paths()
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
        macos::self_update()
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
