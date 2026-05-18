//! Per-OS install/uninstall/update logic.
//!
//! Each platform module owns its service manager integration
//! (systemd, launchd, SCM). This module is the dispatcher: it picks
//! up the host platform via `cfg`, picks the right component path,
//! and routes `install` / `uninstall` / `update` to the platform
//! module.

use anyhow::Result;

use crate::cli::Component;

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
