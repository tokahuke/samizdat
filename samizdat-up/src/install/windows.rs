//! Windows install/uninstall/update, via the Service Control Manager.

use anyhow::Result;

use crate::cli::Component;

use super::{InstallOpts, UninstallOpts};

pub(super) fn install(opts: InstallOpts) -> Result<()> {
    let _ = opts;
    anyhow::bail!("windows::install: not implemented yet (see plan, step 5)")
}

pub(super) fn uninstall(opts: UninstallOpts) -> Result<()> {
    let _ = opts;
    anyhow::bail!("windows::uninstall: not implemented yet (see plan, step 5)")
}

pub(super) fn update(component: Option<Component>, to: Option<String>) -> Result<()> {
    let _ = (component, to);
    anyhow::bail!("windows::update: not implemented yet")
}

pub(super) fn list() -> Result<()> {
    anyhow::bail!("windows::list: not implemented yet")
}

pub(super) fn self_update() -> Result<()> {
    anyhow::bail!("windows::self_update: not implemented yet")
}
