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

use crate::cli::{AdminAction, Component};
use crate::daemons::{self, Daemon, launchd_label};
use crate::fetch::{self, DEFAULT_ORIGIN};

use super::{InstallOpts, UninstallOpts, ADMIN_GROUP};

pub(super) fn install(opts: InstallOpts) -> Result<()> {
    require_root()?;

    let version = opts.version.clone().unwrap_or_else(|| "latest".to_owned());
    let origin = opts.from.clone().unwrap_or_else(|| DEFAULT_ORIGIN.to_owned());
    let target = fetch::host_target_triple();

    ensure_admin_group()?;

    let names = opts.component.daemons();
    let as_user = opts.as_user.as_deref();

    for name in &names {
        let d = daemons::by_name(name).expect("known component name");
        install_daemon_binary(&origin, &version, target, d)?;
        ensure_config(d, as_user)?;
        write_plist(d, as_user)?;
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
    crate::daemons::KNOWN_BINARIES
        .iter()
        .filter_map(|name| {
            let p = PathBuf::from(format!("/usr/local/bin/{name}"));
            p.exists().then_some((*name, p))
        })
        .collect()
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

fn ensure_config(d: &Daemon, as_user: Option<&str>) -> Result<()> {
    fs::create_dir_all("/etc/samizdat").context("creating /etc/samizdat")?;
    let data_dir = format!("/var/lib/samizdat/{}", d.name);
    fs::create_dir_all(&data_dir)
        .with_context(|| format!("creating {data_dir}"))?;
    // Mode 2755 (setgid bit on, world-traversable): setgid forces
    // every file the daemon creates here to inherit the data dir's
    // group (samizdat), so admin-token (0640) is group-readable by
    // group members without per-file chgrp dance.
    let mut perms = fs::metadata(&data_dir)?.permissions();
    perms.set_mode(0o2755);
    fs::set_permissions(&data_dir, perms)?;
    chgrp_recursive(&data_dir, ADMIN_GROUP)?;
    let admin_token = format!("{data_dir}/admin-token");
    if Path::new(&admin_token).exists() {
        let mut p = fs::metadata(&admin_token)?.permissions();
        p.set_mode(0o640);
        fs::set_permissions(&admin_token, p)?;
    }
    if let Some(user) = as_user {
        chown_recursive(&data_dir, user)?;
    }
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

fn write_plist(d: &Daemon, as_user: Option<&str>) -> Result<()> {
    let path = plist_path(d);
    let content = daemons::render_launchd_plist(d, as_user);
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

/// Shell out to `chown -R user:staff path`. Group is `staff` because
/// that's the default primary group for interactive users on macOS;
/// matches what `Finder`/`stat` show for files in `~`. The user must
/// already exist.
fn chown_recursive(path: &str, user: &str) -> Result<()> {
    let target = format!("{user}:staff");
    let status = Command::new("chown")
        .args(["-R", target.as_str(), path])
        .status()
        .with_context(|| format!("running chown -R {target} {path}"))?;
    if !status.success() {
        bail!("chown -R {target} {path} exited with {status}");
    }
    Ok(())
}

fn chgrp_recursive(path: &str, group: &str) -> Result<()> {
    let status = Command::new("chgrp")
        .args(["-R", group, path])
        .status()
        .with_context(|| format!("running chgrp -R {group} {path}"))?;
    if !status.success() {
        bail!("chgrp -R {group} {path} exited with {status}");
    }
    Ok(())
}

/// Idempotent group creation via `dscl`. Picks the next free GID
/// >= 500 (the user-range floor on macOS) to avoid colliding with
/// system groups. If the group already exists, returns early.
fn ensure_admin_group() -> Result<()> {
    // dscl returns non-zero when the record does not exist; success
    // means "found", so we treat success as "already there".
    let exists = Command::new("dscl")
        .args([".", "-read", &format!("/Groups/{ADMIN_GROUP}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if exists {
        return Ok(());
    }

    let gid = next_free_gid()?;
    let path = format!("/Groups/{ADMIN_GROUP}");
    let dscl = |args: &[&str]| -> Result<()> {
        let status = Command::new("dscl")
            .args(args)
            .status()
            .with_context(|| format!("running dscl {}", args.join(" ")))?;
        if !status.success() {
            bail!("dscl {} exited with {status}", args.join(" "));
        }
        Ok(())
    };
    dscl(&[".", "-create", &path])?;
    dscl(&[".", "-create", &path, "PrimaryGroupID", &gid.to_string()])?;
    dscl(&[".", "-create", &path, "RealName", "Samizdat admins"])?;
    dscl(&[".", "-create", &path, "Password", "*"])?;
    println!("samizdat-up: created group `{ADMIN_GROUP}` (gid {gid})");
    Ok(())
}

/// Walk existing groups and pick the lowest unused GID >= 500.
/// 500-1000 is the conventional user-group range on macOS; we sit
/// inside it so the group is distinct from Apple's system groups
/// (which use GIDs < 500).
fn next_free_gid() -> Result<u32> {
    let out = Command::new("dscl")
        .args([".", "-list", "/Groups", "PrimaryGroupID"])
        .output()
        .context("listing /Groups for next-free-gid scan")?;
    if !out.status.success() {
        bail!(
            "dscl . -list /Groups PrimaryGroupID exited with {}",
            out.status
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let used: std::collections::BTreeSet<u32> = text
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u32>().ok())
        .collect();
    for candidate in 500u32..u32::MAX {
        if !used.contains(&candidate) {
            return Ok(candidate);
        }
    }
    bail!("no free GID found")
}

pub(super) fn admin(action: AdminAction) -> Result<()> {
    match action {
        AdminAction::Add { user } => {
            require_root()?;
            ensure_admin_group()?;
            let status = Command::new("dseditgroup")
                .args(["-o", "edit", "-a", &user, "-t", "user", ADMIN_GROUP])
                .status()
                .with_context(|| {
                    format!("running dseditgroup -o edit -a {user} -t user {ADMIN_GROUP}")
                })?;
            if !status.success() {
                bail!("dseditgroup add {user} exited with {status}");
            }
            println!(
                "samizdat-up: added `{user}` to `{ADMIN_GROUP}`. \
                 Log out + back in (or `newgrp {ADMIN_GROUP}`) for it to take effect."
            );
        }
        AdminAction::Rm { user } => {
            require_root()?;
            let status = Command::new("dseditgroup")
                .args(["-o", "edit", "-d", &user, "-t", "user", ADMIN_GROUP])
                .status()
                .with_context(|| {
                    format!("running dseditgroup -o edit -d {user} -t user {ADMIN_GROUP}")
                })?;
            if !status.success() {
                bail!("dseditgroup rm {user} exited with {status}");
            }
            println!("samizdat-up: removed `{user}` from `{ADMIN_GROUP}`.");
        }
        AdminAction::List => {
            let out = Command::new("dscl")
                .args([".", "-read", &format!("/Groups/{ADMIN_GROUP}"), "GroupMembership"])
                .output()
                .with_context(|| format!("reading membership of {ADMIN_GROUP}"))?;
            if !out.status.success() {
                println!("samizdat-up: `{ADMIN_GROUP}` group does not exist yet.");
                return Ok(());
            }
            let text = String::from_utf8_lossy(&out.stdout);
            // `dscl . -read /Groups/<g> GroupMembership` prints:
            //   "GroupMembership: user1 user2 ..."
            // or "No such key: GroupMembership" if empty.
            let members = text
                .lines()
                .find_map(|l| l.strip_prefix("GroupMembership:"))
                .map(str::trim)
                .unwrap_or("");
            if members.is_empty() {
                println!("samizdat-up: `{ADMIN_GROUP}` has no members.");
            } else {
                println!("samizdat-up: `{ADMIN_GROUP}` members: {members}");
            }
        }
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
