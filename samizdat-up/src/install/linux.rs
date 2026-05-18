//! Linux install/uninstall, via systemd.
//!
//! Lays daemons out the same way the old `install.sh` scripts did, so
//! upgrade paths between the old shell installer and samizdat-up
//! preserve user state:
//!
//!   binary  /usr/local/bin/samizdat-<role>      (root:root 0755)
//!   cli     /usr/local/bin/samizdat              (root:root 0755)
//!   unit    /etc/systemd/system/samizdat-<role>.service
//!   config  /etc/samizdat/<role>.toml            (preserved across reinstalls)
//!   data    /var/lib/samizdat/<role>/

use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::Component;
use crate::daemons::{self, Daemon};
use crate::fetch::{self, DEFAULT_ORIGIN};

use super::{InstallOpts, UninstallOpts};

pub(super) fn install(opts: InstallOpts) -> Result<()> {
    require_root()?;

    let version = opts.version.clone().unwrap_or_else(|| "latest".to_owned());
    let origin = opts.from.clone().unwrap_or_else(|| DEFAULT_ORIGIN.to_owned());
    let target = fetch::host_target_triple();

    let daemons = opts.component.daemons();

    // Place the daemon binaries first (and their default configs +
    // unit files). The CLI rides along with any daemon install -- it
    // is what users (and post-install hooks) call.
    for name in &daemons {
        let d = daemons::by_name(name).expect("known component name");
        install_daemon_binary(&origin, &version, target, d)?;
        ensure_config(d)?;
        write_unit_file(d)?;
    }

    // The CLI is also pulled in when only `cli` was requested.
    let install_cli = matches!(
        opts.component,
        Component::Cli | Component::Node | Component::Hub | Component::Proxy | Component::All
    );
    if install_cli {
        install_cli_binary(&origin, &version, target)?;
    }

    if opts.no_service || daemons.is_empty() {
        println!(
            "samizdat-up: binaries placed; service registration skipped \
             (--no-service or cli-only)."
        );
        return Ok(());
    }

    // daemon-reload once after all the .service files have been written
    // is cheaper than once per unit and gets us the same effect.
    systemctl(&["daemon-reload"])?;

    for name in &daemons {
        let unit = format!("samizdat-{name}.service");
        systemctl(&["enable", "--now", &unit])?;
    }

    print_post_install(&daemons);

    Ok(())
}

/// Update each installed daemon (or the specified one). Each binary is
/// atomically replaced, then the service is restarted via systemctl.
/// The CLI co-binary is updated too whenever any daemon is updated.
pub(super) fn update(component: Option<Component>, to: Option<String>) -> Result<()> {
    require_root()?;

    let version = to.unwrap_or_else(|| "latest".to_owned());
    let origin = DEFAULT_ORIGIN.to_owned();
    let target = fetch::host_target_triple();

    // Which daemons to consider: either the user-specified component
    // or "every daemon that has a unit file on disk".
    let candidates: Vec<&str> = if let Some(c) = component {
        c.daemons()
    } else {
        daemons::ALL
            .iter()
            .filter(|d| {
                Path::new(&format!("/etc/systemd/system/samizdat-{}.service", d.name)).exists()
            })
            .map(|d| d.name)
            .collect()
    };

    if candidates.is_empty() {
        println!("samizdat-up: no daemons installed; nothing to update.");
        return Ok(());
    }

    for name in &candidates {
        let d = daemons::by_name(name).expect("known component name");
        install_daemon_binary(&origin, &version, target, d)?;
    }
    // Refresh the CLI alongside the daemons; it ships from the same
    // version line.
    install_cli_binary(&origin, &version, target)?;

    for name in &candidates {
        let unit = format!("samizdat-{name}.service");
        systemctl(&["restart", &unit])?;
        println!("samizdat-up: restarted {unit}");
    }

    Ok(())
}

/// Report what is installed. Detects daemons by the presence of their
/// unit file in /etc/systemd/system, and asks systemctl whether each
/// is active. The CLI is detected by the binary on PATH.
pub(super) fn list() -> Result<()> {
    let mut printed = false;
    for d in daemons::ALL {
        let unit_path = format!("/etc/systemd/system/samizdat-{}.service", d.name);
        let bin_path = format!("/usr/local/bin/samizdat-{}", d.name);
        if !Path::new(&unit_path).exists() && !Path::new(&bin_path).exists() {
            continue;
        }
        let active = Command::new("systemctl")
            .args(["is-active", &format!("samizdat-{}", d.name)])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_else(|| "unknown".to_owned());
        println!("samizdat-{:<6} bin={bin_path} state={active}", d.name);
        printed = true;
    }
    let cli_path = "/usr/local/bin/samizdat";
    if Path::new(cli_path).exists() {
        println!("samizdat        bin={cli_path}");
        printed = true;
    }
    if !printed {
        println!("samizdat-up: nothing installed.");
    }
    Ok(())
}

pub(super) fn self_update() -> Result<()> {
    require_root()?;
    let origin = DEFAULT_ORIGIN.to_owned();
    let target = fetch::host_target_triple();
    let fetched = fetch::fetch_file(&origin, "latest", target, "samizdat-up", "samizdat-up")
        .context("fetching new samizdat-up")?;
    let dest = PathBuf::from("/usr/local/bin/samizdat-up");

    // Stage the new binary in a sibling file, run `--version` on it,
    // and only swap if it answers cleanly. Catches mismatched-arch
    // bytes, corrupted downloads, and binaries that link against a
    // libc the host does not have, all of which would otherwise brick
    // the user's samizdat-up.
    let staged = dest.with_extension("samizdat-up-new");
    atomic_write_executable(&staged, &fetched.bytes)
        .with_context(|| format!("staging new samizdat-up at {}", staged.display()))?;
    smoke_test(&staged)
        .with_context(|| format!("rejected new samizdat-up at {}", staged.display()))?;
    fs::rename(&staged, &dest)
        .with_context(|| format!("renaming {} -> {}", staged.display(), dest.display()))?;
    println!("samizdat-up: self-updated -> {}", dest.display());
    Ok(())
}

/// Spawn `<path> --version` and require exit 0 with output that
/// looks like a samizdat-up version line. Tier 1 self-update gate:
/// catches operational corruption (truncation, wrong arch, missing
/// libc symbols) before the binary lands on PATH.
fn smoke_test(path: &Path) -> Result<()> {
    let out = Command::new(path)
        .arg("--version")
        .output()
        .with_context(|| format!("could not exec {}", path.display()))?;
    if !out.status.success() {
        bail!(
            "{} --version exited with {} (stderr: {:?})",
            path.display(),
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.contains("samizdat-up") {
        bail!(
            "{} --version did not identify as samizdat-up: {:?}",
            path.display(),
            stdout
        );
    }
    Ok(())
}

pub(super) fn uninstall(opts: UninstallOpts) -> Result<()> {
    require_root()?;

    let daemons = opts.component.daemons();

    for name in &daemons {
        let unit = format!("samizdat-{name}.service");
        // Best-effort stop + disable. systemctl returns non-zero if
        // the unit is already gone; that is fine.
        let _ = Command::new("systemctl").args(["stop", &unit]).status();
        let _ = Command::new("systemctl")
            .args(["disable", &unit])
            .status();
        let unit_path = format!("/etc/systemd/system/{unit}");
        let _ = fs::remove_file(&unit_path);

        let bin_path = format!("/usr/local/bin/samizdat-{name}");
        let _ = fs::remove_file(&bin_path);
    }

    // CLI is shared across daemons; only remove it if the user asked
    // for `all` or `cli`.
    if matches!(opts.component, Component::Cli | Component::All) {
        let _ = fs::remove_file("/usr/local/bin/samizdat");
    }

    let _ = Command::new("systemctl").args(["daemon-reload"]).status();

    if opts.purge {
        // Wipe configs and data. Series private keys go too.
        let _ = fs::remove_dir_all("/etc/samizdat");
        let _ = fs::remove_dir_all("/var/lib/samizdat");
        println!("samizdat-up: purged /etc/samizdat and /var/lib/samizdat.");
    } else {
        println!(
            "samizdat-up: uninstalled. /etc/samizdat and /var/lib/samizdat preserved.\n\
             To wipe them too, re-run with --purge."
        );
    }

    Ok(())
}

fn install_daemon_binary(
    origin: &str,
    version: &str,
    target: &str,
    d: &Daemon,
) -> Result<()> {
    let fetched =
        fetch::fetch_file(origin, version, target, d.name, d.bin).context("fetching daemon")?;
    let dest = PathBuf::from(format!("/usr/local/bin/{}", d.bin));
    atomic_write_executable(&dest, &fetched.bytes)
        .with_context(|| format!("installing {} from {}", dest.display(), fetched.source))?;
    println!("samizdat-up: installed {} -> {}", d.bin, dest.display());
    Ok(())
}

fn install_cli_binary(origin: &str, version: &str, target: &str) -> Result<()> {
    // The CLI ships under the `node` component in the dist tree (the
    // shell installers historically downloaded it from there too).
    let fetched = fetch::fetch_file(origin, version, target, "node", "samizdat")
        .context("fetching samizdat CLI")?;
    let dest = PathBuf::from("/usr/local/bin/samizdat");
    atomic_write_executable(&dest, &fetched.bytes)
        .with_context(|| format!("installing CLI from {}", fetched.source))?;
    println!("samizdat-up: installed samizdat CLI -> {}", dest.display());
    Ok(())
}

fn ensure_config(d: &Daemon) -> Result<()> {
    fs::create_dir_all("/etc/samizdat").context("creating /etc/samizdat")?;
    fs::create_dir_all(format!("/var/lib/samizdat/{}", d.name))
        .with_context(|| format!("creating /var/lib/samizdat/{}", d.name))?;
    let path = PathBuf::from(format!("/etc/samizdat/{}.toml", d.name));
    if path.exists() {
        // Preserve user edits. The original install.sh used
        // `cp --no-clobber`; same intent here.
        return Ok(());
    }
    fs::write(&path, d.default_config)
        .with_context(|| format!("writing default config to {}", path.display()))?;
    println!(
        "samizdat-up: wrote default config -> {} \
         (edit and `systemctl restart` to apply changes)",
        path.display()
    );
    Ok(())
}

fn write_unit_file(d: &Daemon) -> Result<()> {
    let path = PathBuf::from(format!("/etc/systemd/system/samizdat-{}.service", d.name));
    let content = daemons::render_systemd_unit(d);
    fs::write(&path, content)
        .with_context(|| format!("writing systemd unit to {}", path.display()))?;
    println!("samizdat-up: wrote unit file -> {}", path.display());
    Ok(())
}

fn atomic_write_executable(dest: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| format!("mkdir -p {}", parent.display()))?;
    }
    let tmp = dest.with_extension("samizdat-up-tmp");
    {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o755)
            .open(&tmp)
            .with_context(|| format!("opening {} for write", tmp.display()))?;
        f.write_all(bytes)
            .with_context(|| format!("writing {}", tmp.display()))?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, dest)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), dest.display()))?;
    let mut perms = fs::metadata(dest)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(dest, perms)?;
    Ok(())
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .args(args)
        .status()
        .with_context(|| format!("running `systemctl {}`", args.join(" ")))?;
    if !status.success() {
        bail!("`systemctl {}` exited with {}", args.join(" "), status);
    }
    Ok(())
}

fn require_root() -> Result<()> {
    // SAFETY: getuid is always safe to call; no inputs.
    let uid = unsafe { libc_getuid() };
    if uid != 0 {
        bail!("samizdat-up must be run as root (try `sudo samizdat-up ...`).");
    }
    Ok(())
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn print_post_install(daemons: &[&str]) {
    println!();
    for name in daemons {
        let unit = format!("samizdat-{name}.service");
        println!("samizdat-{name} installed.  systemd: {unit} (active)");
        println!("    stop:    sudo systemctl stop {unit}");
        println!("    start:   sudo systemctl start {unit}");
        println!("    restart: sudo systemctl restart {unit}");
        println!("    check:   systemctl status {unit}");
        println!("    remove:  sudo samizdat-up uninstall {name}");
        println!();
    }
}
