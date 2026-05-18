//! Per-daemon metadata + render functions for system-service unit
//! files / plists / SCM args.
//!
//! Kept platform-agnostic on purpose: the unit-file content is just a
//! function of the daemon metadata, so it can be unit-tested against
//! golden files on any host. The cfg-gated `install::{linux,macos,
//! windows}` modules pick the right renderer and do the side-effectful
//! "write the file, call systemctl/launchctl/sc" half.

/// Per-daemon metadata. Adding a fourth daemon someday means
/// appending one constant to [`ALL`].
pub struct Daemon {
    /// Short name: "node" | "hub" | "proxy". Drives URL component
    /// path, unit name, config filename.
    pub name: &'static str,
    /// Daemon binary basename ("samizdat-node" etc.).
    pub bin: &'static str,
    /// Description for the systemd unit + launchd ProgramArguments
    /// comments.
    pub description: &'static str,
    /// Default TOML config content. Written only when the config does
    /// not already exist, so a user's local edits survive a reinstall.
    pub default_config: &'static str,
}

pub const NODE: Daemon = Daemon {
    name: "node",
    bin: "samizdat-node",
    description: "Samizdat Node",
    default_config: include_str!("../defaults/node.toml"),
};

pub const HUB: Daemon = Daemon {
    name: "hub",
    bin: "samizdat-hub",
    description: "Samizdat Hub",
    default_config: include_str!("../defaults/hub.toml"),
};

pub const PROXY: Daemon = Daemon {
    name: "proxy",
    bin: "samizdat-proxy",
    description: "Samizdat Proxy",
    default_config: include_str!("../defaults/proxy.toml"),
};

pub const ALL: &[&Daemon] = &[&NODE, &HUB, &PROXY];

pub fn by_name(name: &str) -> Option<&'static Daemon> {
    ALL.iter().copied().find(|d| d.name == name)
}

/// Reverse-DNS label used as the launchd service identifier. Used for
/// the plist filename, the `Label` key, and `launchctl` arguments.
pub fn launchd_label(d: &Daemon) -> String {
    format!("com.samizdat.{}", d.name)
}

/// Render the launchd plist for one daemon. Pure function; output is
/// snapshot-tested against `tests/golden/com.samizdat.<name>.plist`.
///
/// Notes:
///   - `RunAtLoad` makes the service start the moment launchctl loads
///     the plist, mirroring systemd's `enable --now`.
///   - `KeepAlive` ensures the daemon comes back after a crash.
///   - Paths match the Linux layout (/usr/local/bin, /etc/samizdat,
///     /var/lib/samizdat) so users can administer a Mac install with
///     the same paths they would on a Linux box.
pub fn render_launchd_plist(d: &Daemon) -> String {
    let label = launchd_label(d);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20   <key>Label</key>\n\
         \x20   <string>{label}</string>\n\
         \x20   <key>ProgramArguments</key>\n\
         \x20   <array>\n\
         \x20       <string>/usr/local/bin/{bin}</string>\n\
         \x20       <string>--config</string>\n\
         \x20       <string>/etc/samizdat/{role}.toml</string>\n\
         \x20   </array>\n\
         \x20   <key>RunAtLoad</key>\n\
         \x20   <true/>\n\
         \x20   <key>KeepAlive</key>\n\
         \x20   <true/>\n\
         \x20   <key>StandardOutPath</key>\n\
         \x20   <string>/var/log/{bin}-stdout.log</string>\n\
         \x20   <key>StandardErrorPath</key>\n\
         \x20   <string>/var/log/{bin}-stderr.log</string>\n\
         \x20   <key>EnvironmentVariables</key>\n\
         \x20   <dict>\n\
         \x20       <key>RUST_BACKTRACE</key>\n\
         \x20       <string>1</string>\n\
         \x20   </dict>\n\
         </dict>\n\
         </plist>\n",
        label = label,
        bin = d.bin,
        role = d.name,
    )
}

/// Render the systemd unit file for one daemon. Pure function; output
/// is snapshot-tested against `tests/golden/samizdat-<name>.service`.
pub fn render_systemd_unit(d: &Daemon) -> String {
    format!(
        "[Unit]\n\
         Description={description}\n\
         After=network.target\n\
         StartLimitIntervalSec=0\n\
         \n\
         [Service]\n\
         Type=simple\n\
         Restart=always\n\
         RestartSec=1\n\
         User=root\n\
         Environment=RUST_BACKTRACE=1\n\
         ExecStart=/usr/local/bin/{bin} --config /etc/samizdat/{role}.toml\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        description = d.description,
        bin = d.bin,
        role = d.name,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_for_node_matches_golden() {
        let actual = render_systemd_unit(&NODE);
        let golden = include_str!("../tests/golden/samizdat-node.service");
        assert_eq!(
            actual, golden,
            "unit file drift; if intentional, update the golden file"
        );
    }

    #[test]
    fn systemd_unit_for_hub_matches_golden() {
        let actual = render_systemd_unit(&HUB);
        let golden = include_str!("../tests/golden/samizdat-hub.service");
        assert_eq!(actual, golden);
    }

    #[test]
    fn systemd_unit_for_proxy_matches_golden() {
        let actual = render_systemd_unit(&PROXY);
        let golden = include_str!("../tests/golden/samizdat-proxy.service");
        assert_eq!(actual, golden);
    }

    #[test]
    fn launchd_plist_for_node_matches_golden() {
        let actual = render_launchd_plist(&NODE);
        let golden = include_str!("../tests/golden/com.samizdat.node.plist");
        assert_eq!(actual, golden, "plist drift; update the golden if intentional");
    }

    #[test]
    fn launchd_plist_for_hub_matches_golden() {
        let actual = render_launchd_plist(&HUB);
        let golden = include_str!("../tests/golden/com.samizdat.hub.plist");
        assert_eq!(actual, golden);
    }

    #[test]
    fn launchd_plist_for_proxy_matches_golden() {
        let actual = render_launchd_plist(&PROXY);
        let golden = include_str!("../tests/golden/com.samizdat.proxy.plist");
        assert_eq!(actual, golden);
    }

    #[test]
    fn launchd_label_uses_reverse_dns() {
        assert_eq!(launchd_label(&NODE), "com.samizdat.node");
        assert_eq!(launchd_label(&HUB), "com.samizdat.hub");
        assert_eq!(launchd_label(&PROXY), "com.samizdat.proxy");
    }
}
