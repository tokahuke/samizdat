//! macOS install/uninstall/update, via launchd.

use anyhow::Result;

use crate::cli::Component;

use super::{InstallOpts, UninstallOpts};

pub(super) fn install(opts: InstallOpts) -> Result<()> {
    let _ = opts;
    anyhow::bail!("macos::install: not implemented yet (see plan, step 6)")
}

pub(super) fn uninstall(opts: UninstallOpts) -> Result<()> {
    let _ = opts;
    anyhow::bail!("macos::uninstall: not implemented yet (see plan, step 6)")
}

pub(super) fn update(component: Option<Component>, to: Option<String>) -> Result<()> {
    let _ = (component, to);
    anyhow::bail!("macos::update: not implemented yet")
}

pub(super) fn list() -> Result<()> {
    anyhow::bail!("macos::list: not implemented yet")
}

pub(super) fn self_update() -> Result<()> {
    anyhow::bail!("macos::self_update: not implemented yet")
}
