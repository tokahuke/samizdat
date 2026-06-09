//! Script-based DNS-01 provider. Shells out to an operator-supplied
//! binary (or a pair of binaries, one per action) so users can integrate
//! with DNS providers we do not ship a built-in for. The script receives
//! the zone, record name, value, and the action via environment
//! variables; the rest of the proxy's environment is inherited verbatim
//! so the script can read provider-specific credentials the operator
//! already configured (typically in `proxy.env`).
//!
//! Two shapes are supported:
//!
//! * Single-command form: one binary handles both create and delete, distinguishing by
//!   `SAMIZDAT_DNS_ACTION=set|delete`.
//! * Pair form: separate binaries for create and delete; the action env var is not set in
//!   this shape.

use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use tokio::{process::Command, time::timeout};
use tracing::{debug, warn};

use super::{DnsError, DnsProvider, TxtHandle};

/// Maximum number of characters from a failing script's stderr that gets
/// embedded in a `DnsError::Provider`. Bounded to keep operator log spam
/// in check when a script misbehaves.
const STDERR_LIMIT: usize = 512;

/// Internal layout of the script provider. The single-command form
/// stores the same path in both `set` and `delete` slots.
pub struct Script {
    set: PathBuf,
    delete: PathBuf,
    timeout: Duration,
}

impl Script {
    /// Build a single-command script provider. The same binary is
    /// invoked for both create and delete; `SAMIZDAT_DNS_ACTION` is set
    /// to `set` or `delete` to disambiguate.
    pub fn single(command: PathBuf, timeout: Duration) -> Self {
        Script {
            set: command.clone(),
            delete: command,
            timeout,
        }
    }

    /// Build a pair-form script provider. The `set` binary runs for
    /// create, `delete` runs for delete. `SAMIZDAT_DNS_ACTION` is set
    /// either way (pair-form scripts can ignore it).
    pub fn pair(set: PathBuf, delete: PathBuf, timeout: Duration) -> Self {
        Script {
            set,
            delete,
            timeout,
        }
    }

    /// Run one of the configured binaries with the standard env-var
    /// contract. The proxy's environment is inherited verbatim by
    /// default (tokio's `Command` matches std::process semantics); we
    /// layer the four `SAMIZDAT_DNS_*` vars on top.
    async fn run(
        &self,
        binary: &PathBuf,
        action: &str,
        zone: &str,
        record_name: &str,
        value: &str,
    ) -> Result<(), DnsError> {
        let mut cmd = Command::new(binary);
        cmd.env("SAMIZDAT_DNS_ZONE", zone)
            .env("SAMIZDAT_DNS_NAME", record_name)
            .env("SAMIZDAT_DNS_VALUE", value)
            .env("SAMIZDAT_DNS_ACTION", action);
        // Capture both streams so we can surface stderr on failure
        // without writing the script's chatter to the proxy's own
        // stdout/stderr.
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        debug!(?binary, action, zone, record_name, "invoking DNS-01 script");

        let child = cmd.spawn().map_err(|err| {
            DnsError::Provider(format!(
                "failed to spawn DNS-01 script {}: {err}",
                binary.display()
            ))
        })?;

        let wait = child.wait_with_output();
        let output = match timeout(self.timeout, wait).await {
            Ok(Ok(output)) => output,
            Ok(Err(err)) => {
                return Err(DnsError::Provider(format!(
                    "DNS-01 script {} failed to run: {err}",
                    binary.display()
                )));
            }
            Err(_) => {
                // wait_with_output consumed the child handle, so we
                // cannot send an explicit kill here. The child's stdio
                // pipes get dropped as `output` goes out of scope,
                // which closes them; most scripts terminate on the
                // next write. The OS will reap whatever remains when
                // the proxy exits. Log loudly so the operator notices.
                warn!(
                    ?binary,
                    timeout_seconds = self.timeout.as_secs(),
                    "DNS-01 script timed out; abandoning child"
                );
                return Err(DnsError::Provider(format!(
                    "script timed out after {}s",
                    self.timeout.as_secs()
                )));
            }
        };

        if !output.status.success() {
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_owned());
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            let trimmed = stderr_str.trim();
            let bounded: String = trimmed.chars().take(STDERR_LIMIT).collect();
            return Err(DnsError::Provider(format!(
                "script failed with exit {code}: stderr={bounded}"
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl DnsProvider for Script {
    async fn set_txt(
        &self,
        zone: &str,
        record_name: &str,
        value: &str,
    ) -> Result<TxtHandle, DnsError> {
        let set_path = self.set.clone();
        self.run(&set_path, "set", zone, record_name, value).await?;
        // Encode both pieces in the handle so `remove_txt` can recover
        // them without keeping per-call state on the provider. Null
        // byte is safe because record names and TXT values are ASCII
        // per the ACME and DNS specs.
        Ok(TxtHandle(format!("{record_name}\x00{value}")))
    }

    async fn remove_txt(&self, zone: &str, handle: TxtHandle) -> Result<(), DnsError> {
        let raw = handle.0;
        let Some((record_name, value)) = raw.split_once('\x00') else {
            return Err(DnsError::Provider(
                "malformed handle: expected <name>\\0<value>".to_owned(),
            ));
        };
        let delete_path = self.delete.clone();
        self.run(&delete_path, "delete", zone, record_name, value)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Write a shell script to a unique path under the system temp dir
    /// and mark it executable. Returns the path so the caller can clean
    /// up at end of test. We use the process id and a counter to avoid
    /// collisions between concurrent tests in the same binary.
    #[cfg(unix)]
    fn write_script(name: &str, body: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "samizdat-script-test-{}-{}-{}.sh",
            std::process::id(),
            n,
            name
        ));
        let mut f = std::fs::File::create(&path).expect("create temp script");
        f.write_all(body.as_bytes()).expect("write script body");
        f.sync_all().expect("sync");
        drop(f);
        let mut perms = std::fs::metadata(&path).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("chmod");
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn single_command_success() {
        let script = write_script("ok", "#!/bin/sh\necho \"$SAMIZDAT_DNS_VALUE\"\nexit 0\n");
        let provider = Script::single(script.clone(), Duration::from_secs(5));
        let result = provider
            .set_txt("example.com", "_acme-challenge.example.com", "abc123")
            .await;
        let _ = std::fs::remove_file(&script);
        let handle = result.expect("set_txt should succeed");
        assert_eq!(handle.0, "_acme-challenge.example.com\x00abc123");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn single_command_failure_surfaces_stderr() {
        let script = write_script("fail", "#!/bin/sh\necho 'kaboom-marker' >&2\nexit 1\n");
        let provider = Script::single(script.clone(), Duration::from_secs(5));
        let result = provider
            .set_txt("example.com", "_acme-challenge.example.com", "abc123")
            .await;
        let _ = std::fs::remove_file(&script);
        match result {
            Err(DnsError::Provider(msg)) => {
                assert!(
                    msg.contains("kaboom-marker"),
                    "expected stderr substring in error, got: {msg}"
                );
                assert!(
                    msg.contains("exit 1"),
                    "expected exit code in error, got: {msg}"
                );
            }
            other => panic!("expected DnsError::Provider, got {other:?}"),
        }
    }
}

/// `proxy.toml` configuration for the script provider.
#[derive(Debug, serde_derive::Deserialize)]
pub struct ScriptTopology {
    /// Absolute path to a binary invoked with `SAMIZDAT_DNS_ACTION` set
    /// to `set` or `delete`. Mutually exclusive with `set` and `delete`.
    #[serde(default)]
    pub command: Option<std::path::PathBuf>,
    /// Absolute path to a binary invoked only for create. Used together
    /// with `delete`.
    #[serde(default)]
    pub set: Option<std::path::PathBuf>,
    /// Absolute path to a binary invoked only for delete. Used together
    /// with `set`.
    #[serde(default)]
    pub delete: Option<std::path::PathBuf>,
    /// Maximum wall-clock seconds the script may run before the proxy
    /// gives up on it. Default 30.
    #[serde(default = "default_script_timeout")]
    pub timeout_seconds: u64,
}

fn default_script_timeout() -> u64 {
    30
}

#[typetag::deserialize(name = "script")]
impl crate::dns::ProviderConfig for ScriptTopology {
    fn resolve(&self) -> anyhow::Result<Box<dyn crate::dns::DnsProvider>> {
        let timeout = std::time::Duration::from_secs(self.timeout_seconds);
        match (self.command.clone(), self.set.clone(), self.delete.clone()) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                anyhow::bail!(
                    "script provider must use EITHER `command` OR `set`+`delete`, not both"
                )
            }
            (Some(c), None, None) => Ok(Box::new(Script::single(c, timeout))),
            (None, Some(s), Some(d)) => Ok(Box::new(Script::pair(s, d, timeout))),
            (None, Some(_), None) | (None, None, Some(_)) => {
                anyhow::bail!("script provider needs both `set` and `delete` paths")
            }
            (None, None, None) => {
                anyhow::bail!("script provider needs either `command` or `set`+`delete`")
            }
        }
    }
}
