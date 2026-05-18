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
}
