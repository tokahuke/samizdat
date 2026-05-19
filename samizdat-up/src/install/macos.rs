//! macOS install/uninstall/update, via launchd.
//!
//! Layout parallels the Linux install on purpose (same paths for
//! binaries, configs, and data), so administering a Mac install uses
//! the same paths users learn on Linux. Only the service-manager call
//! is different: launchctl in place of systemctl, a plist in place of
//! a systemd unit file.
//!
//!   binary  /usr/local/bin/samizdat-<role>          (root:wheel 0755)
//!   cli     /usr/local/bin/samizdat                  (root:wheel 0755)
//!   plist   /Library/LaunchDaemons/com.samizdat.<role>.plist (root:wheel 0644)
//!   config  /etc/samizdat/<role>.toml                (preserved across reinstalls)
//!   data    /var/lib/samizdat/<role>/

use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::cli::Component;
use crate::daemons::{self, Daemon, launchd_label};
use crate::fetch::{self, DEFAULT_ORIGIN};

use super::{InstallOpts, UninstallOpts};

pub(super) fn install(opts: InstallOpts) -> Result<()> {
    require_root()?;

    let version = opts.version.clone().unwrap_or_else(|| "latest".to_owned());
    let origin = opts.from.clone().unwrap_or_else(|| DEFAULT_ORIGIN.to_owned());
    let target = fetch::host_target_triple();

    let names = opts.component.daemons();

    for name in &names {
        let d = daemons::by_name(name).expect("known component name");
        install_daemon_binary(&origin, &version, target, d)?;
        ensure_config(d)?;
        write_plist(d)?;
    }

    let install_cli = matches!(
        opts.component,
        Component::Cli | Component::Node | Component::Hub | Component::Proxy | Component::All
    );
    if install_cli {
        install_cli_binary(&origin, &version, target)?;
    }

    if opts.no_service || names.is_empty() {
        println!(
            "samizdat-up: binaries placed; service registration skipped \
             (--no-service or cli-only)."
        );
        return Ok(());
    }

    for name in &names {
        let d = daemons::by_name(name).expect("known");
        let plist = plist_path(d);
        // `bootstrap` errors if the service is already loaded, so we
        // unload first. On a fresh install there is nothing loaded to
        // bootout, which makes launchctl print "Boot-out failed: 3:
        // No such process" -- noisy but harmless. Swallow that output
        // so the install log only shows real events.
        let label = launchd_label(d);
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("system/{label}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        launchctl(&[
            "bootstrap",
            "system",
            plist.to_str().context("plist path utf-8")?,
        ])?;
        launchctl(&["enable", &format!("system/{label}")])?;
    }

    print_post_install(&names);

    Ok(())
}

pub(super) fn uninstall(opts: UninstallOpts) -> Result<()> {
    require_root()?;

    for name in opts.component.daemons() {
        let d = daemons::by_name(name).expect("known");
        let label = launchd_label(d);
        // bootout stops the daemon and removes it from the launchd
        // session. Non-zero exit when the service was not registered
        // is fine; the stderr "Boot-out failed: 3: No such process"
        // is noise that should not pollute the uninstall log.
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("system/{label}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_file(plist_path(d));
        let _ = fs::remove_file(format!("/usr/local/bin/samizdat-{name}"));
    }

    if matches!(opts.component, Component::Cli | Component::All) {
        let _ = fs::remove_file("/usr/local/bin/samizdat");
    }

    if opts.purge {
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

pub(super) fn update(component: Option<Component>, to: Option<String>) -> Result<()> {
    require_root()?;
    let version = to.unwrap_or_else(|| "latest".to_owned());
    let origin = DEFAULT_ORIGIN.to_owned();
    let target = fetch::host_target_triple();

    let candidates: Vec<&str> = if let Some(c) = component {
        c.daemons()
    } else {
        daemons::ALL
            .iter()
            .filter(|d| plist_path(d).exists())
            .map(|d| d.name)
            .collect()
    };

    if candidates.is_empty() {
        println!("samizdat-up: no daemons installed; nothing to update.");
        return Ok(());
    }

    for name in &candidates {
        let d = daemons::by_name(name).expect("known");
        install_daemon_binary(&origin, &version, target, d)?;
    }
    install_cli_binary(&origin, &version, target)?;

    for name in &candidates {
        let d = daemons::by_name(name).expect("known");
        let label = launchd_label(d);
        // `kickstart -k` stops + restarts the named service. Works on
        // anything launchctl has loaded.
        launchctl(&["kickstart", "-k", &format!("system/{label}")])?;
        println!("samizdat-up: restarted {label}");
    }
    Ok(())
}

pub(super) fn list() -> Result<()> {
    let mut any = false;
    for d in daemons::ALL {
        let plist = plist_path(d);
        let bin = format!("/usr/local/bin/samizdat-{}", d.name);
        if !plist.exists() && !Path::new(&bin).exists() {
            continue;
        }
        let label = launchd_label(d);
        let state = match Command::new("launchctl")
            .args(["print", &format!("system/{label}")])
            .output()
        {
            Ok(o) if o.status.success() => "loaded",
            _ => "not-loaded",
        };
        println!("samizdat-{:<6} bin={bin} state={state}", d.name);
        any = true;
    }
    let cli = "/usr/local/bin/samizdat";
    if Path::new(cli).exists() {
        println!("samizdat        bin={cli}");
        any = true;
    }
    if !any {
        println!("samizdat-up: nothing installed.");
    }
    Ok(())
}

pub(super) fn installed_binary_paths() -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    for d in daemons::ALL {
        let p = PathBuf::from(format!("/usr/local/bin/{}", d.bin));
        if p.exists() {
            out.push((d.bin, p));
        }
    }
    let cli = PathBuf::from("/usr/local/bin/samizdat");
    if cli.exists() {
        out.push(("samizdat", cli));
    }
    let up = PathBuf::from("/usr/local/bin/samizdat-up");
    if up.exists() {
        out.push(("samizdat-up", up));
    }
    out
}

pub(super) fn self_update() -> Result<()> {
    require_root()?;
    let origin = DEFAULT_ORIGIN.to_owned();
    let target = fetch::host_target_triple();
    let fetched = fetch::fetch_file(&origin, "latest", target, "samizdat-up", "samizdat-up")
        .context("fetching new samizdat-up")?;
    let dest = PathBuf::from("/usr/local/bin/samizdat-up");

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

fn install_daemon_binary(origin: &str, version: &str, target: &str, d: &Daemon) -> Result<()> {
    let fetched = fetch::fetch_file(origin, version, target, d.name, d.bin)?;
    let dest = PathBuf::from(format!("/usr/local/bin/{}", d.bin));
    atomic_write_executable(&dest, &fetched.bytes)
        .with_context(|| format!("installing {} from {}", dest.display(), fetched.source))?;
    println!("samizdat-up: installed {} -> {}", d.bin, dest.display());
    Ok(())
}

fn install_cli_binary(origin: &str, version: &str, target: &str) -> Result<()> {
    let fetched = fetch::fetch_file(origin, version, target, "node", "samizdat")?;
    let dest = PathBuf::from("/usr/local/bin/samizdat");
    atomic_write_executable(&dest, &fetched.bytes)
        .with_context(|| format!("installing CLI from {}", fetched.source))?;
    println!("samizdat-up: installed samizdat CLI -> {}", dest.display());
    Ok(())
}

fn ensure_config(d: &Daemon) -> Result<()> {
    fs::create_dir_all("/etc/samizdat").context("creating /etc/samizdat")?;
    let data_dir = format!("/var/lib/samizdat/{}", d.name);
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating {data_dir}"))?;
    // 0755 so the world-readable `read-token` is reachable without
    // sudo. Admin secrets stay 0600 in the node; widening the
    // directory's traversal bit alone doesn't expose them.
    let mut perms = fs::metadata(&data_dir)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&data_dir, perms)?;
    let path = PathBuf::from(format!("/etc/samizdat/{}.toml", d.name));
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, d.default_config)
        .with_context(|| format!("writing default config to {}", path.display()))?;
    println!(
        "samizdat-up: wrote default config -> {} \
         (edit and `launchctl kickstart -k system/com.samizdat.{}` to apply changes)",
        path.display(),
        d.name
    );
    Ok(())
}

fn write_plist(d: &Daemon) -> Result<()> {
    let path = plist_path(d);
    let content = daemons::render_launchd_plist(d);
    fs::write(&path, content)
        .with_context(|| format!("writing launchd plist to {}", path.display()))?;
    // launchd refuses to load plists that are not owned by root or are
    // group/world-writable. Set 0644 explicitly even though that is
    // usually the default umask result.
    let mut perms = fs::metadata(&path)?.permissions();
    perms.set_mode(0o644);
    fs::set_permissions(&path, perms)?;
    println!("samizdat-up: wrote plist -> {}", path.display());
    Ok(())
}

fn plist_path(d: &Daemon) -> PathBuf {
    PathBuf::from(format!(
        "/Library/LaunchDaemons/com.samizdat.{}.plist",
        d.name
    ))
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
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, dest)?;
    let mut perms = fs::metadata(dest)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(dest, perms)?;
    Ok(())
}

fn launchctl(args: &[&str]) -> Result<()> {
    let status = Command::new("launchctl")
        .args(args)
        .status()
        .with_context(|| format!("running `launchctl {}`", args.join(" ")))?;
    if !status.success() {
        bail!("`launchctl {}` exited with {}", args.join(" "), status);
    }
    Ok(())
}

fn require_root() -> Result<()> {
    // SAFETY: getuid takes no inputs and is always safe to call.
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
        let label = format!("com.samizdat.{name}");
        println!("samizdat-{name} installed.  launchd: {label} (loaded + enabled)");
        println!("    stop:    sudo launchctl bootout system/{label}");
        println!("    start:   sudo launchctl bootstrap system /Library/LaunchDaemons/{label}.plist");
        println!("    restart: sudo launchctl kickstart -k system/{label}");
        println!("    check:   sudo launchctl print system/{label}");
        println!("    remove:  sudo samizdat-up uninstall {name}");
        println!();
    }
}
