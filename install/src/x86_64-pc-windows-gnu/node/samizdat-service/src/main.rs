//! Windows service wrapper around `samizdat-node.exe`.
//!
//! Registered with the Service Control Manager (SCM) by the NSIS
//! installer. On start the wrapper:
//!
//!   1. Parses `--data=<dir>` from its own argv (populated by SCM from
//!      the registered `binPath`) and stashes it in `DATA_DIR`.
//!   2. Hands control to SCM via `service_dispatcher::start`.
//!   3. Inside `service_main`, registers a control handler, reports
//!      `Running`, and enters a supervise-loop that spawns
//!      `samizdat-node.exe` (resolved relative to the wrapper's own
//!      directory), waits for either the child to exit or a stop /
//!      shutdown request, then either restarts or reports `Stopped`.
//!
//! Log files are opened in append mode in `<DATA_DIR>\stdout.log` and
//! `<DATA_DIR>\stderr.log` so prior runs are preserved across restarts.

#[cfg(not(target_os = "windows"))]
fn main() -> ! {
    panic!("samizdat-service only builds for Windows targets")
}

#[cfg(target_os = "windows")]
mod service {
    use std::env;
    use std::ffi::OsString;
    use std::fs::{self, OpenOptions};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::OnceLock;
    use std::sync::mpsc::{self, RecvTimeoutError, Sender};
    use std::time::Duration;

    use windows_service::define_windows_service;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{
        self, ServiceControlHandlerResult,
    };
    use windows_service::service_dispatcher;

    /// Service name. MUST match the name passed to `sc.exe create` by the
    /// NSIS installer (`SERVICE_NAME` define in `installer.nsi`). Single
    /// source of truth on this side; the installer is the source on the
    /// other.
    const SERVICE_NAME: &str = "SamizdatNode";

    /// Default data directory when none is passed in `binPath`. Matches
    /// what the NSIS installer creates.
    const DEFAULT_DATA_DIR: &str = r"C:\ProgramData\Samizdat\Node";

    /// Polling interval for the child-exit / shutdown-request loop.
    const SUPERVISE_POLL: Duration = Duration::from_millis(500);

    /// Cool-down between child restart attempts.
    const RESTART_BACKOFF: Duration = Duration::from_secs(2);

    /// Data dir parsed from `binPath` args in `main()`, before SCM hands
    /// control over to `service_main`. Stored in a `OnceLock` because the
    /// SCM-invoked `service_main` cannot easily receive parameters from
    /// `main()`.
    static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    pub fn run() -> Result<(), windows_service::Error> {
        // The installer sets `binPath= "...\samizdat-service.exe --data=<dir>"`.
        // SCM populates argv from that string, so std::env::args sees it.
        DATA_DIR
            .set(parse_data_dir(env::args().collect::<Vec<_>>()))
            .ok();

        // Hand control to SCM. This call blocks until the service stops.
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
    }

    fn parse_data_dir(args: Vec<String>) -> PathBuf {
        for arg in &args {
            if let Some(rest) = arg.strip_prefix("--data=") {
                return PathBuf::from(rest);
            }
        }
        // Fall back to the conventional install location so the service
        // can still come up if `binPath` is mis-configured.
        PathBuf::from(DEFAULT_DATA_DIR)
    }

    fn service_main(_arguments: Vec<OsString>) {
        if let Err(err) = run_service() {
            eprintln!("samizdat-service main loop exited with error: {err}");
        }
    }

    fn run_service() -> Result<(), windows_service::Error> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        let event_handler = make_event_handler(shutdown_tx);
        let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

        // Tell SCM we're up. The "Starting" state has a ~30s timeout on
        // Windows; if `Running` is not reported within it the service is
        // marked failed.
        status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        // Resolve the node binary relative to OUR exe, not via PATH or
        // CWD. SCM launches services with `cwd = C:\Windows\System32`,
        // and the install location is not on PATH by default.
        let node_path = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("samizdat-node.exe")))
            .unwrap_or_else(|| PathBuf::from("samizdat-node.exe"));

        let data_dir = DATA_DIR
            .get()
            .cloned()
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR));

        // Best-effort: make sure the data dir exists before we try to
        // open log files inside it.
        let _ = fs::create_dir_all(&data_dir);

        supervise(&node_path, &data_dir, &shutdown_rx);

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
        move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    // Best-effort: the receiver may already be gone.
                    let _ = shutdown_tx.send(());
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        }
    }

    /// Run-forever loop: launch the node, wait for either it to exit or a
    /// shutdown signal, restart on exit unless shutting down.
    fn supervise(
        node_path: &std::path::Path,
        data_dir: &std::path::Path,
        shutdown_rx: &mpsc::Receiver<()>,
    ) {
        loop {
            if shutdown_rx.try_recv().is_ok() {
                return;
            }

            let stdout = open_log(data_dir, "stdout.log");
            let stderr = open_log(data_dir, "stderr.log");

            let spawn_result = Command::new(node_path)
                .arg("--data")
                .arg(data_dir)
                .env("RUST_BACKTRACE", "1")
                .stdout(stdout)
                .stderr(stderr)
                .spawn();

            match spawn_result {
                Ok(child) => {
                    let stopped_for_shutdown = wait_or_shutdown(child, shutdown_rx);
                    if stopped_for_shutdown {
                        return;
                    }
                }
                Err(err) => {
                    eprintln!(
                        "Failed to spawn {}: {err}",
                        node_path.display()
                    );
                }
            }

            // Cool-down before respawning. Interruptible by a shutdown
            // signal so we don't sit here while SCM is waiting.
            match shutdown_rx.recv_timeout(RESTART_BACKOFF) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// Returns `true` if the wait ended because a shutdown was requested.
    /// Returns `false` if the child exited on its own.
    fn wait_or_shutdown(mut child: Child, shutdown_rx: &mpsc::Receiver<()>) -> bool {
        loop {
            if shutdown_rx.try_recv().is_ok() {
                // Best-effort graceful kill. samizdat-node uses tokio
                // and gets force-killed; we don't have a clean stop
                // signal across the process boundary yet.
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
            match child.try_wait() {
                Ok(Some(_status)) => return false,
                Ok(None) => std::thread::sleep(SUPERVISE_POLL),
                Err(err) => {
                    eprintln!("error waiting for samizdat-node: {err}");
                    return false;
                }
            }
        }
    }

    /// Opens a log file in append mode so prior runs are preserved
    /// across restarts of the supervised child. Falls back to inheriting
    /// the parent stdio if the file can't be created (e.g. permissions
    /// on the data dir).
    fn open_log(data_dir: &std::path::Path, name: &str) -> Stdio {
        let path = data_dir.join(name);
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => Stdio::from(file),
            Err(err) => {
                eprintln!(
                    "could not open {}: {err}; inheriting stdio instead",
                    path.display()
                );
                Stdio::inherit()
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn main() -> Result<(), windows_service::Error> {
    service::run()
}
