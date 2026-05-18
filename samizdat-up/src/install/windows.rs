//! Windows install/uninstall/update + the SCM service wrapper.
//!
//! On Windows samizdat-up plays two roles in one binary:
//!
//!   1. **Installer / CLI**: the user-facing entry. `install` lays out
//!      binaries, registers a service with the Service Control
//!      Manager via `sc.exe create`, and starts it.
//!
//!   2. **SCM service wrapper**: SCM is configured to launch the
//!      service with `binPath = "...\samizdat-up.exe" daemon <role>`.
//!      When SCM starts that process, `samizdat-up daemon <role>`
//!      hands control to `windows_service::service_dispatcher::start`
//!      and then supervises the actual daemon binary in a child
//!      process, forwarding Stop/Shutdown signals from SCM into a
//!      child-kill.
//!
//! Daemons (`samizdat-node`, `samizdat-hub`, `samizdat-proxy`) are
//! therefore SCM-unaware -- the wrapper here speaks the SCM protocol
//! on their behalf. This keeps the daemons cross-platform-pure (no
//! Windows-only main() branch) and concentrates the SCM lifecycle in
//! one place.
//!
//! Layout on disk:
//!
//!   binary    C:\Program Files\Samizdat\samizdat-<role>.exe
//!   cli       C:\Program Files\Samizdat\samizdat.exe
//!   wrapper   C:\Program Files\Samizdat\samizdat-up.exe
//!   config    C:\ProgramData\Samizdat\<role>.toml
//!   data      C:\ProgramData\Samizdat\<role>\
//!   logs      C:\ProgramData\Samizdat\<role>\stdout.log + stderr.log

use anyhow::{Context, Result, bail};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::Component;
use crate::daemons::{self, Daemon};
use crate::fetch::{self, DEFAULT_ORIGIN};

use super::{InstallOpts, UninstallOpts};

const SAMIZDAT_UP_EXE: &str = "samizdat-up.exe";

fn install_dir() -> PathBuf {
    let pf = std::env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".to_owned());
    PathBuf::from(pf).join("Samizdat")
}

fn data_root() -> PathBuf {
    let pd = std::env::var("PROGRAMDATA").unwrap_or_else(|_| r"C:\ProgramData".to_owned());
    PathBuf::from(pd).join("Samizdat")
}

fn service_name(d: &Daemon) -> String {
    format!("samizdat-{}", d.name)
}

fn binary_path(d: &Daemon) -> PathBuf {
    install_dir().join(format!("{}.exe", d.bin))
}

fn cli_path() -> PathBuf {
    install_dir().join("samizdat.exe")
}

fn wrapper_path() -> PathBuf {
    install_dir().join(SAMIZDAT_UP_EXE)
}

fn config_path(d: &Daemon) -> PathBuf {
    data_root().join(format!("{}.toml", d.name))
}

fn data_path(d: &Daemon) -> PathBuf {
    data_root().join(d.name)
}

pub(super) fn install(opts: InstallOpts) -> Result<()> {
    require_admin()?;

    let version = opts.version.clone().unwrap_or_else(|| "latest".to_owned());
    let origin = opts.from.clone().unwrap_or_else(|| DEFAULT_ORIGIN.to_owned());
    let target = fetch::host_target_triple();

    fs::create_dir_all(install_dir())
        .with_context(|| format!("creating {}", install_dir().display()))?;
    fs::create_dir_all(data_root())
        .with_context(|| format!("creating {}", data_root().display()))?;

    // The wrapper (this binary's own current_exe) goes into the install
    // dir alongside the daemon, so SCM's binPath can resolve cleanly
    // even if the user installed samizdat-up from a different
    // location.
    place_self()?;

    let names = opts.component.daemons();

    for name in &names {
        let d = daemons::by_name(name).expect("known component name");
        install_daemon_binary(&origin, &version, target, d)?;
        ensure_config(d)?;
    }

    let install_cli = matches!(
        opts.component,
        Component::Cli | Component::Node | Component::Hub | Component::Proxy | Component::All
    );
    if install_cli {
        install_cli_binary(&origin, &version, target)?;
    }

    if opts.no_service || names.is_empty() {
        println!("samizdat-up: binaries placed; service registration skipped.");
        return Ok(());
    }

    for name in &names {
        let d = daemons::by_name(name).expect("known");
        sc_create(d)?;
        sc(&["start", &service_name(d)])?;
    }

    print_post_install(&names);
    Ok(())
}

pub(super) fn uninstall(opts: UninstallOpts) -> Result<()> {
    require_admin()?;

    for name in opts.component.daemons() {
        let d = daemons::by_name(name).expect("known");
        let svc = service_name(d);
        // Best-effort stop, then delete. sc returns non-zero if the
        // service is already gone; that is fine for an uninstall.
        let _ = Command::new("sc.exe").args(["stop", &svc]).status();
        let _ = Command::new("sc.exe").args(["delete", &svc]).status();
        let _ = fs::remove_file(binary_path(d));
    }

    if matches!(opts.component, Component::Cli | Component::All) {
        let _ = fs::remove_file(cli_path());
    }

    if opts.purge {
        let _ = fs::remove_dir_all(data_root());
        // Keep the install_dir intact for samizdat-up itself.
        println!(
            "samizdat-up: purged {}.",
            data_root().display()
        );
    } else {
        println!(
            "samizdat-up: uninstalled. {} preserved.\n\
             To wipe data too, re-run with --purge.",
            data_root().display()
        );
    }

    Ok(())
}

pub(super) fn update(component: Option<Component>, to: Option<String>) -> Result<()> {
    require_admin()?;
    let version = to.unwrap_or_else(|| "latest".to_owned());
    let origin = DEFAULT_ORIGIN.to_owned();
    let target = fetch::host_target_triple();

    let candidates: Vec<&str> = if let Some(c) = component {
        c.daemons()
    } else {
        daemons::ALL
            .iter()
            .filter(|d| service_registered(d))
            .map(|d| d.name)
            .collect()
    };

    if candidates.is_empty() {
        println!("samizdat-up: no daemons installed; nothing to update.");
        return Ok(());
    }

    // Stop running services BEFORE replacing the binary -- Windows
    // refuses to overwrite a file that is mapped by a running process.
    for name in &candidates {
        let d = daemons::by_name(name).expect("known");
        let _ = Command::new("sc.exe").args(["stop", &service_name(d)]).status();
    }

    for name in &candidates {
        let d = daemons::by_name(name).expect("known");
        install_daemon_binary(&origin, &version, target, d)?;
    }
    install_cli_binary(&origin, &version, target)?;

    for name in &candidates {
        let d = daemons::by_name(name).expect("known");
        sc(&["start", &service_name(d)])?;
        println!("samizdat-up: restarted {}", service_name(d));
    }
    Ok(())
}

pub(super) fn list() -> Result<()> {
    let mut any = false;
    for d in daemons::ALL {
        if !binary_path(d).exists() && !service_registered(d) {
            continue;
        }
        let state = sc_query_state(d).unwrap_or_else(|| "unknown".to_owned());
        println!(
            "samizdat-{:<6} bin={} state={}",
            d.name,
            binary_path(d).display(),
            state
        );
        any = true;
    }
    if cli_path().exists() {
        println!("samizdat        bin={}", cli_path().display());
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
        let p = binary_path(d);
        if p.exists() {
            out.push((d.bin, p));
        }
    }
    let cli = cli_path();
    if cli.exists() {
        out.push(("samizdat", cli));
    }
    let up = wrapper_path();
    if up.exists() {
        out.push(("samizdat-up", up));
    }
    out
}

pub(super) fn self_update() -> Result<()> {
    require_admin()?;
    let origin = DEFAULT_ORIGIN.to_owned();
    let target = fetch::host_target_triple();
    let fetched =
        fetch::fetch_file(&origin, "latest", target, "samizdat-up", SAMIZDAT_UP_EXE)
            .context("fetching new samizdat-up.exe")?;
    let dest = wrapper_path();

    // Stage the new exe + smoke-test it before parking the running
    // one. If the new binary is corrupt or wrong-arch, we bail with
    // the old samizdat-up.exe untouched.
    let staged = dest.with_extension("exe.new");
    atomic_write(&staged, &fetched.bytes)
        .with_context(|| format!("staging new samizdat-up at {}", staged.display()))?;
    smoke_test(&staged)
        .with_context(|| format!("rejected new samizdat-up at {}", staged.display()))?;

    // Windows can't overwrite a running .exe; park the current one
    // first. The kernel keeps the running mapping until process exit,
    // but the path is now free for the new binary.
    let parked = dest.with_extension("exe.old");
    let _ = fs::remove_file(&parked);
    if dest.exists() {
        fs::rename(&dest, &parked)
            .with_context(|| format!("parking {}", dest.display()))?;
    }
    fs::rename(&staged, &dest)
        .with_context(|| format!("renaming {} -> {}", staged.display(), dest.display()))?;
    println!("samizdat-up: self-updated -> {}", dest.display());
    println!("(previous samizdat-up.exe parked at {}; remove later if you like)", parked.display());
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

// ---------- helpers ----------

fn install_daemon_binary(
    origin: &str,
    version: &str,
    target: &str,
    d: &Daemon,
) -> Result<()> {
    let file = format!("{}.exe", d.bin);
    let fetched = fetch::fetch_file(origin, version, target, d.name, &file)?;
    let dest = binary_path(d);
    atomic_write(&dest, &fetched.bytes)
        .with_context(|| format!("installing {} from {}", dest.display(), fetched.source))?;
    println!("samizdat-up: installed {} -> {}", file, dest.display());
    Ok(())
}

fn install_cli_binary(origin: &str, version: &str, target: &str) -> Result<()> {
    let fetched = fetch::fetch_file(origin, version, target, "node", "samizdat.exe")?;
    let dest = cli_path();
    atomic_write(&dest, &fetched.bytes)
        .with_context(|| format!("installing CLI from {}", fetched.source))?;
    println!("samizdat-up: installed samizdat CLI -> {}", dest.display());
    Ok(())
}

/// Place a copy of the running samizdat-up.exe at
/// `<install_dir>\samizdat-up.exe` so SCM's binPath has a stable
/// location to point at. No-op if the source and destination resolve
/// to the same file (e.g. the user already installed via brew/curl
/// then re-ran samizdat-up from that location).
fn place_self() -> Result<()> {
    let here = std::env::current_exe().context("locating current samizdat-up.exe")?;
    let dest = wrapper_path();
    if let Ok(canon_here) = here.canonicalize() {
        if let Ok(canon_dest) = dest.canonicalize() {
            if canon_here == canon_dest {
                return Ok(());
            }
        }
    }
    let bytes = fs::read(&here)
        .with_context(|| format!("reading {}", here.display()))?;
    atomic_write(&dest, &bytes)?;
    println!("samizdat-up: wrapper at {}", dest.display());
    Ok(())
}

fn ensure_config(d: &Daemon) -> Result<()> {
    fs::create_dir_all(data_path(d))
        .with_context(|| format!("creating {}", data_path(d).display()))?;
    let path = config_path(d);
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, d.default_config)
        .with_context(|| format!("writing default config to {}", path.display()))?;
    println!(
        "samizdat-up: wrote default config -> {} (edit and `sc stop && sc start` to apply)",
        path.display()
    );
    Ok(())
}

/// Atomic-ish write: write to <dest>.tmp, then rename. On Windows
/// `rename` is atomic within the same volume.
fn atomic_write(dest: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("samizdat-up-tmp");
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all().ok();
    }
    // On Windows, fs::rename fails if dest exists; remove first.
    let _ = fs::remove_file(dest);
    fs::rename(&tmp, dest)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), dest.display()))?;
    Ok(())
}

fn sc(args: &[&str]) -> Result<()> {
    let status = Command::new("sc.exe")
        .args(args)
        .status()
        .with_context(|| format!("running `sc.exe {}`", args.join(" ")))?;
    if !status.success() {
        bail!("`sc.exe {}` exited with {}", args.join(" "), status);
    }
    Ok(())
}

/// Issue `sc.exe create ...` for the daemon. The binPath points at our
/// wrapper invocation, NOT directly at the daemon binary -- the
/// wrapper is what speaks SCM.
fn sc_create(d: &Daemon) -> Result<()> {
    // `binPath= "..."` uses the quirky `key= value` (space after `=`)
    // syntax. The value itself must be a single quoted string; the
    // inner quotes around the exe path are escaped so a path with
    // spaces (the default `C:\Program Files\Samizdat\...`) survives.
    let wrapper = wrapper_path();
    let bin_value = format!(
        "\"{}\" daemon {}",
        wrapper.display(),
        d.name
    );
    sc(&[
        "create",
        &service_name(d),
        "binPath=",
        &bin_value,
        "DisplayName=",
        &format!("Samizdat {}", d.description),
        "start=",
        "auto",
    ])?;
    let _ = sc(&[
        "description",
        &service_name(d),
        &format!("Samizdat {} (managed by samizdat-up).", d.description),
    ]);
    Ok(())
}

fn service_registered(d: &Daemon) -> bool {
    Command::new("sc.exe")
        .args(["query", &service_name(d)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn sc_query_state(d: &Daemon) -> Option<String> {
    let out = Command::new("sc.exe")
        .args(["query", &service_name(d)])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("STATE") {
            // "STATE              : 4  RUNNING" -> take everything after the colon
            if let Some(after) = rest.split_once(':') {
                return Some(after.1.split_whitespace().last()?.to_owned());
            }
        }
    }
    None
}

fn require_admin() -> Result<()> {
    // Windows admin detection is messy via Rust stdlib alone; the SCM
    // calls below will fail with ERROR_ACCESS_DENIED when not elevated,
    // which surfaces via `sc.exe` as a clear non-zero exit. Rather
    // than re-implementing IsUserAnAdmin, lean on that.
    Ok(())
}

fn print_post_install(names: &[&str]) {
    println!();
    for n in names {
        let svc = format!("samizdat-{n}");
        println!("samizdat-{n} installed.  SCM service: {svc}");
        println!("    stop:    sc.exe stop {svc}");
        println!("    start:   sc.exe start {svc}");
        println!("    check:   sc.exe query {svc}");
        println!("    remove:  samizdat-up uninstall {n}");
        println!();
    }
}

// ============================================================
// SCM-side: `samizdat-up daemon <role>` runs HERE.
// ============================================================

use std::ffi::OsString;
use std::process::{Child, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use windows_service::define_windows_service;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

/// Poll interval for the child-exit / shutdown-request loop.
const SUPERVISE_POLL: Duration = Duration::from_millis(500);
/// Cool-down between child restart attempts.
const RESTART_BACKOFF: Duration = Duration::from_secs(2);

/// Which daemon this service-mode process supervises. Set by
/// `run_as_service` before handing control to SCM, since the
/// SCM-invoked `service_main` does not take parameters from the
/// surrounding scope.
static DAEMON: OnceLock<&'static Daemon> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

pub(super) fn run_as_service(component: Component) -> Result<()> {
    let role_name = component
        .daemons()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("daemon subcommand needs a daemon component (node/hub/proxy)"))?;
    let d = daemons::by_name(role_name).expect("known component");
    DAEMON.set(d).ok();
    service_dispatcher::start(service_name(d), ffi_service_main)
        .context("registering with SCM service dispatcher")
}

fn service_main(_args: Vec<OsString>) {
    if let Err(err) = run_service() {
        eprintln!("samizdat-up daemon: service main exited with error: {err}");
    }
}

fn run_service() -> Result<(), windows_service::Error> {
    let d = DAEMON
        .get()
        .copied()
        .expect("DAEMON set before service_dispatcher::start");

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    let handler = make_event_handler(shutdown_tx);
    let status_handle = service_control_handler::register(service_name(d), handler)?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    supervise(d, &shutdown_rx);

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}

fn make_event_handler(
    shutdown_tx: Sender<()>,
) -> impl Fn(ServiceControl) -> ServiceControlHandlerResult {
    move |control_event| match control_event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = shutdown_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    }
}

fn supervise(d: &Daemon, shutdown_rx: &mpsc::Receiver<()>) {
    let bin = binary_path(d);
    let config = config_path(d);
    let logs = data_path(d);

    loop {
        if shutdown_rx.try_recv().is_ok() {
            return;
        }

        let stdout = open_log(&logs, "stdout.log");
        let stderr = open_log(&logs, "stderr.log");

        let spawn = Command::new(&bin)
            .arg("--config")
            .arg(&config)
            .env("RUST_BACKTRACE", "1")
            .stdout(stdout)
            .stderr(stderr)
            .spawn();

        match spawn {
            Ok(child) => {
                let stopped_for_shutdown = wait_or_shutdown(child, shutdown_rx);
                if stopped_for_shutdown {
                    return;
                }
            }
            Err(err) => {
                eprintln!("failed to spawn {}: {err}", bin.display());
            }
        }

        // Interruptible cool-down between restarts.
        match shutdown_rx.recv_timeout(RESTART_BACKOFF) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// `true` if the wait ended because shutdown was requested, `false`
/// if the child exited on its own.
fn wait_or_shutdown(mut child: Child, shutdown_rx: &mpsc::Receiver<()>) -> bool {
    loop {
        if shutdown_rx.try_recv().is_ok() {
            let _ = child.kill();
            let _ = child.wait();
            return true;
        }
        match child.try_wait() {
            Ok(Some(_)) => return false,
            Ok(None) => std::thread::sleep(SUPERVISE_POLL),
            Err(err) => {
                eprintln!("error waiting for {}: {err}", child.id());
                return false;
            }
        }
    }
}

fn open_log(data_dir: &Path, name: &str) -> Stdio {
    let path = data_dir.join(name);
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => Stdio::from(f),
        Err(err) => {
            eprintln!(
                "could not open {}: {err}; inheriting stdio instead",
                path.display()
            );
            Stdio::inherit()
        }
    }
}
