# Extras: the non-Rust pieces

The Rust workspace is the core, but several other folders ship with the
project. This document maps what each one is, what it expects, and what
audit findings landed against it.

For *how to operate* the testbed (recreate from scratch, debug a
broken release, what's TF-managed vs out-of-band), see
[`operations.md`](operations.md).

## Sibling repos

- **`../samizdat-blog`** is a Hugo blog whose built `public/` is
  published as a Samizdat collection (`fGfgc7ib...`, ttl 120s). The
  project dogfoods its own content distribution: docs, blog posts, and
  the install site itself ship over Samizdat.

There are no other `samizdat-*` sibling folders.

## In-repo extras

### `blockchain/`

Solidity contracts deployed on Polygon (chain ID 137).

- `SamizdatIdentityStorage` is a key-value store mapping name -> `Entry
  { entity, owner, ttl, extraData }`.
- `SamizdatIdentityV1` is the operator contract that mediates writes
  (`registerWithTtl`, `transfer`) and forwards reads.
- `SamizdatStorage.json` and `SamizdatIdentityV1.json` are the ABIs the
  Rust node parses via `ethers-rs` in
  `node/src/identity_dapp/mod.rs`.

The node verifies the configured RPC reports the expected chain ID
before trusting any read, but does not validate state with Merkle
proofs; a malicious RPC can still serve stale or forked state. Treat
self-hosted RPC as the supported configuration for high-stakes
lookups; treat the public RPC default as best-effort.

### `js/`

`samizdatjs` is a small TypeScript library that runs inside browser
pages served by a node. The page's origin is the node itself, so the
library uses **relative URLs** (`fetch(route)` with no host); a
hard-coded `http://localhost:4510` would break on any non-default
port and would expose the page to `/etc/hosts`-rebinding shenanigans.

Authentication piggybacks on the browser's `Referer` header: as long
as the page was loaded from the local node, the node can extract the
entity from the referer and look up its access rights. For elevated
rights the library opens a popup at `/_register` and waits on a
custom DOM event; the popup must be on the same origin so it can
dispatch into the parent. The popup-close path rejects with an error
rather than hanging forever, which is what the previous version did.

### `install/`

The cross-platform install path is now owned by a single Rust
binary, `samizdat-up` (see the dedicated crate at the workspace
root). What lives in `install/` is the minimum that has to survive
that crate:

- `install/get-samizdat/` -- the public release submodule. Holds
  `.Samizdat.priv` (the publishing series private key, gitignored)
  and `dist/` (the artifact tree the proxy serves).
- `install/src/install.sh.template` -- the bootstrap shim served at
  `~get-samizdat/<version>/install.sh`. Detects OS+arch, downloads
  the right `samizdat-up` binary, places it in `/usr/local/bin/`.
- `install/src/x86_64-unknown-linux-gnu/testbed/proxy.toml.template`
  -- the testbed's `proxy.toml` with `${DOMAIN}` and
  `${PROXY_OWNER_EMAIL}` placeholders, applied by
  `bootstrap-testbed.yaml` after `samizdat-up install proxy` writes
  the default config.
- `install/src/rust/MacOSX14.5.tar.xz` -- the macOS SDK used by
  cargo-zigbuild for cross-compiling to `aarch64-apple-darwin` from
  the GitHub Actions ubuntu-latest runner.

What used to live here -- the Python+Docker builder, the per-role
shell installers, the NSIS installer, the standalone
`samizdat-service` SCM wrapper -- has all been subsumed into
`samizdat-up`. See its `defaults/` (config templates) and `tests/golden/`
(systemd unit + launchd plist snapshots).

### `samizdat-up/`

Cross-platform installer + updater. Replaces three separate install
codepaths with one Rust binary that has `cfg`-gated branches for
systemd / launchd / SCM. The user-visible surface:

    sudo samizdat-up install <node|hub|proxy|cli|all> [--version V] [--no-service] [--from URL]
    sudo samizdat-up uninstall <component> [--purge]
    sudo samizdat-up update [<component>] [--to V]
    samizdat-up list
    samizdat-up versions [--remote]
    sudo samizdat-up self-update

`install` places the binary at `/usr/local/bin/`, writes a default
config (preserving any existing one), registers the daemon with the
host's service manager, and starts it. The CLI (`samizdat`) ships
alongside any daemon install -- it is the administrative tool the
daemon's hooks call. Lifecycle (start/stop/restart/status) is the
OS's job after install; samizdat-up does not re-implement those
verbs.

Per-OS state is canonical for the platform:

| Path | Linux + macOS | Windows |
|---|---|---|
| data | `/var/lib/samizdat/<role>` | `C:\ProgramData\Samizdat\<role>` |
| config | `/etc/samizdat/<role>.toml` | `C:\ProgramData\Samizdat\<role>.toml` |
| binary | `/usr/local/bin/samizdat-<role>` | `C:\Program Files\Samizdat\samizdat-<role>.exe` |
| unit | `/etc/systemd/system/...` or `/Library/LaunchDaemons/com.samizdat.<role>.plist` | SCM (no on-disk unit) |

The render functions for unit files / plists live in
`samizdat-up/src/daemons.rs` as pure functions, snapshot-tested
against `samizdat-up/tests/golden/`. The side-effectful parts
(`fs::write`, `systemctl`, `launchctl`, `sc.exe`) live in the
per-OS modules under `samizdat-up/src/install/`.

### `simulate-net/`

Local network simulator. Reads a `network.ryan` topology description
(nodes, hubs, and which nodes connect to which hubs), allocates
free loopback ports for every interface, compiles each binary with
`cargo build`, spawns the processes, and waits.

Quality-of-life behavior:

- Paths are anchored to the repo root (`Path(__file__).parent.parent`),
  so the script works regardless of CWD.
- Ports come from `socket.bind((::1, 0))` rather than a fixed base, so
  there are no collisions with whatever else is on the host.
- `cargo build` exit codes are checked; a failed build aborts the
  simulation instead of silently running a stale binary.
- The "press any key to continue" prompt is skipped when stdin is not
  a TTY or when `SAMIZDAT_SIM_NONINTERACTIVE` is set in the env.
- `--release` and `--config <path>` flags via argparse.

### `terraform/`

DigitalOcean infrastructure-as-code for the public testbed droplet.
Uses the Terraform Cloud backend (the `cloud {}` block in `main.tf`)
for state.

Resources:

- One `digitalocean_droplet` running Ubuntu 22.04 (`s-1vcpu-1gb`, nyc3).
- DNS A/AAAA for `testbed.hubfederation.com` + `proxy.hubfederation.com`
  pointing at it.
- An ED25519 keypair (`tls_private_key`) whose public half is uploaded
  to DO as the droplet's SSH key and whose private half is pushed to
  the GitHub repo as the `TESTBED_SSH_KEY` action secret.
- `TESTBED_HOST` and `PROXY_OWNER_EMAIL` action secrets, also managed
  by TF.

Cloud-init on the droplet is intentionally minimal: it opens the
firewall ports the services bind, creates `/etc/samizdat` and the
per-service state dirs under `/var/lib/samizdat`, and installs fish
as the login shell. **No samizdat binaries** are pulled at boot; the
bootstrap workflow does that over SSH.

Required variables (set in TF Cloud workspace or local `.tfvars`):
`do_token`, `github_token`, `github_owner`, `proxy_owner_email`.

### `.github/workflows/`

Three workflows, all `workflow_dispatch`:

- **`publish-get-samizdat.yaml`** (the "release" workflow): cross-
  compiles all five binaries (samizdat-up, samizdat, samizdat-node,
  samizdat-hub, samizdat-proxy) for the four supported targets
  (linux x86_64, windows x86_64, darwin aarch64; hub + proxy linux
  only) via cargo-zigbuild on `ubuntu-latest`. Lays them into the
  `install/get-samizdat/dist/<version|latest>/<triple>/<component>/`
  tree, also writes a `install.sh` bootstrap shim per version, then
  signs + announces a fresh edition of the `get-samizdat` series
  using the `GET_SAMIZDAT_PRIV` secret. Waits until the testbed has
  eager-fetched the new content.
- **`bootstrap-testbed.yaml`**: one-shot post-`terraform apply`. Builds
  samizdat-up + daemons locally on the runner, scp's them to the
  testbed as a `file://` dist tree, runs `samizdat-up install` for
  each role, overlays the testbed-specific proxy.toml (https=true,
  real domain), subscribes the testbed's node to `get-samizdat` and
  `samizdat-blog`. Run once per droplet creation.
- **`update-testbed.yaml`**: trivial follow-up. ssh's into the droplet
  and runs `samizdat-up update`, which pulls the latest binaries
  from the testbed's own copy of the get-samizdat collection and
  restarts services. Run after a successful `publish-get-samizdat`.

### Releasing end-to-end

A complete release goes roughly:

1. `./release.sh 0.2.0` from a clean `stable` checkout. Bumps the
   workspace version, commits, tags, pushes, dispatches
   `publish-get-samizdat.yaml`.
2. The publish workflow cross-compiles every target, signs the new
   edition, announces it. The testbed eager-fetches and starts
   serving the new binaries.
3. `gh workflow run update-testbed.yaml` upgrades the testbed itself
   onto the binaries it now serves.

End users on Linux/macOS get the new version next time they run:

    sudo samizdat-up update

or, for a fresh install:

    curl -fsSL https://proxy.hubfederation.com/~get-samizdat/latest/install.sh | sudo bash
    sudo samizdat-up install node

## Findings landed against the extras

### Fixed in this pass

- **JS lib**: relative URLs, no more hardcoded port; fixed `postHub`
  double-JSON encoding; corrected `/_seriesowner` -> `/_seriesowners`
  in `postEdition`; corrected missing `/` in `deleteSubscription`;
  popup close on auth flow now rejects the promise instead of
  hanging; removed stray `console.log`.
- **Solidity**: emit `OperatorChanged`/`OwnerChanged` on storage,
  `OwnerChanged`/`PriceChanged`/`Deprecated`/`Withdrawn` on V1;
  `setOperator` and `changeOwner` reject `address(0)`; new
  `changeOwner` on storage (was missing, leaving a lost deployer
  key as a permanent brick); `withdraw` uses `.call{value:}()`
  instead of legacy `.transfer` so multisig owners don't hit the
  2300-gas limit; `deprecate` rejects `address(0)` superseding
  contract.
- **install/.../proxy/install.sh**: typo `nproxyode.toml` -> `proxy.toml`.
- **install/build.yaml**: proxy export now points at
  `proxy/build-install.sh` (was pointing at `hub/build-install.sh`).
- **.github/workflows/deploy-testbed.yaml**: `actions/checkout@v4`
  (was `@v2`, deprecated).
- **simulate-net/__main__.py**: workspace-root path anchoring,
  OS-assigned free ports, cargo-build return-code checks, CLI flags,
  TTY-aware prompt.
- **Windows**: full rewrite of `samizdat-service` and `installer.nsi`
  (correct SCM lifecycle, control handler, append-mode logs,
  binPath quoting, uninstaller stops+deletes the service, Add/Remove
  Programs entry, ask-before-wipe data dir). NSIS now runs inside
  the rust Docker image as part of the normal release build and
  `samizdat-installer.exe` ships as an artifact alongside the .exe
  binaries.

### Deferred

See [`deferred.md`](deferred.md) — extras items live under the
**Blockchain**, **Install pipeline**, **Windows**, **CI**, and
**Terraform** sections.
