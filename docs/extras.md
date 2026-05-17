# Extras: the non-Rust pieces

The Rust workspace is the core, but several other folders ship with the
project. This document maps what each one is, what it expects, and what
audit findings landed against it.

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

Multi-platform build pipeline written in Python (Poetry-managed). The
top-level orchestrator is `python -m builder`, which reads
`build.yaml` and:

- builds Docker images defined under `images:`,
- runs builder commands defined under `builders:` inside those
  images,
- and exports artifacts defined under `exports:` from the build
  containers back to the host.

Supported targets:

- `x86_64-unknown-linux-gnu`: node, hub, proxy. Each ships a
  systemd unit and an `install.sh`.
- `aarch64-apple-darwin`: node + cli, plus a Homebrew formula
  template at `install/src/aarch64-apple-darwin/homebrew/Samizdat.rb`.
- `x86_64-pc-windows-gnu`: node + cli + service wrapper +
  `samizdat-installer.exe` (NSIS).

The `install/get-samizdat/` directory is itself a Samizdat collection
holding the build artifacts and a static install page. After a
release build, `postbuild.sh` syncs the produced artifacts into that
collection and pushes a new edition.

### Cutting a release

The repo-root `release.sh` is the orchestrator. From a clean
`stable` checkout:

    ./release.sh 0.2.0           # bumps, commits, tags v0.2.0, pushes
    ./release.sh --dry-run 0.2.0 # just print the steps

It bumps `workspace.package.version` in the root `Cargo.toml` (all
crates inherit via `version.workspace = true`), commits the bump
and the refreshed `Cargo.lock`, tags `vX.Y.Z`, pushes, and
dispatches `build-artifacts.yaml` against the new tag via
`gh workflow run`. The GitHub Action then runs the python builder,
which propagates `VERSION` into the builder containers (the NSIS
installer picks it up via `/DVERSION=$VERSION` and writes
`DisplayVersion` to Add/Remove Programs), builds all artifacts, and
finally `postbuild.sh` syncs them into the `get-samizdat`
submodule and pushes the release commit there.

The cargo registry + git index are cached across CI runs via
`actions/cache` + a bind-mount into the rust builder container, so
warm rebuilds skip dep downloads.

### `install/src/x86_64-pc-windows-gnu/` (the Windows path)

`samizdat-service` is a small Rust binary that registers with the
Service Control Manager (SCM) and supervises `samizdat-node.exe`.

Lifecycle:

1. `installer.nsi` runs `sc.exe create SamizdatNode binPath= "...
   samizdat-service.exe --data=..."`.
2. SCM launches `samizdat-service.exe` with that argv.
3. `main()` parses `--data=<dir>` from argv, then hands control to
   `service_dispatcher::start`.
4. `service_main` registers a control handler, reports `Running`,
   and enters a supervise-loop. Each iteration spawns
   `samizdat-node.exe` (resolved relative to the wrapper, not via
   PATH), waits for it to exit OR for a `Stop`/`Shutdown` from
   SCM, restarts if the child exited on its own, exits cleanly
   otherwise.

The installer also writes an `Add/Remove Programs` entry pointing at
`$INSTDIR\uninstall.exe`. The uninstaller stops + deletes the service,
removes the binaries, and asks the user before wiping
`C:\ProgramData\Samizdat\Node` (which holds series keys and
bookmarks).

`install/src/x86_64-pc-windows-gnu/node/build.sh` is a standalone
helper for iterating on the `.nsi` locally; the canonical end-to-end
build is `install/src/rust/build.sh` (run inside the Docker image),
which produces the binaries and then invokes `makensis` to package
them.

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

Four workflows, all `workflow_dispatch`:

- **`build-artifacts.yaml`**: cuts release artifacts for all targets
  via the Python builder pipeline and pushes them into the
  `get-samizdat` submodule.
- **`bootstrap-testbed.yaml`**: one-shot post-`terraform apply` job
  that builds Linux x86_64 binaries, scp's them + systemd units +
  configs + the `samizdat-self-update` script onto the droplet,
  enables the three services, and subscribes the testbed's node to
  the `get-samizdat` and `samizdat-blog` collections. Run this once
  per droplet creation.
- **`update-testbed.yaml`**: trivial follow-up that ssh's into the
  droplet and runs `samizdat-self-update`. The script pulls the
  latest binaries from the testbed's own subscription to
  `get-samizdat` (the network feeds itself) and atomically restarts
  the services. Run this after publishing a new `get-samizdat`
  edition from your laptop.
- (Historically there was a `deploy-testbed.yaml` that tried to do
  rsync-from-CI deploys on push-to-stable. It was 100 lines of
  commented-out aspirational code; it has been deleted in favor of
  the bootstrap + update split above.)

### Releasing end-to-end

A complete release goes roughly:

1. `./release.sh 0.2.0` from a clean `stable` checkout. Bumps the
   workspace version, commits, tags, pushes, dispatches
   `build-artifacts.yaml`.
2. Wait for the action: it rebuilds all targets and pushes the new
   artifacts into the `get-samizdat` submodule on `main`.
3. Publish a new edition of the `get-samizdat` collection from your
   laptop (the `.Samizdat.priv` key lives in
   `install/get-samizdat/`, gitignored): `samizdat collection update`
   inside that directory.
4. Wait for the testbed's subscription to refresh (or force it), then
   run `update-testbed.yaml`. The droplet pulls the new binaries
   from itself via samizdat and restarts.

### `.github/workflows/`

Two workflows: `build-artifacts.yaml` (release artifacts) and
`deploy-testbed.yaml` (testbed deploy on push-to-stable, mostly
commented out). Both run on `workflow_dispatch`, no `pull_request`
exposure.

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
