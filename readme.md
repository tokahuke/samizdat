# Samizdat: your content, available.

[![Continuous Integration](https://github.com/tokahuke/samizdat/actions/workflows/test-samizdat-up.yaml/badge.svg?branch=main)](https://github.com/tokahuke/samizdat/actions/workflows/test-samizdat-up.yaml)
![Version 0.3.3](https://img.shields.io/badge/version-0.3.3-informational)

## Website

Samizdat is pulling itself by its bootstraps!
https://samizdat.hubfederation.com

## Donate

If you support this work, consider donating using crypto

| Currency | Address                                      |
|----------|----------------------------------------------|
| `XMR`    | `86YcEFJSQXfZbPhjpDpabb5raQjVLWAfji3eMGebbj6QJnk1wXfgfqx9pgqURUWqMbjW7mNTC79guNEEsGPKJbRGKxEkrAN` |
| `BTC`    | `bc1qseae89zr4z2lkl82nvvr6c9sl97agshapzeag5` |
| `ETH`    | `0xba89B660eB6f5D894830C9273a5Dfb8dDc170cff` |


## Introduction

Samizdat is a peer-to-peer network for publishing content without a server. Most servers are run by _them_; this one isn't run by anyone.

### Warning

Still a proof of concept. Three caveats:

1. Don't rely on the network or your content staying up; keep backups.
2. Expect breaking changes.
3. Expect vulnerabilities. Don't put anything sensitive on it yet.

> How to make this warning go away? Contribute. I'm one person.

## Project goals

Samizdat (Russian for "self-publishing") wants to be a decentralized application that can:

1. Serve a public static site with no hosting service. Content lives on the publisher's device and in the caches of people who visited the site. (READY)

2. Give resources a human-friendly, content-addressed URL. (IN CONSTRUCTION)

3. Hide who is asking for what: only the device serving and the device asking learn anything about the content or its metadata. (BY DESIGN)

4. Do all of the above easily. GUIs, mobile apps, conveniences welcome. (IN CONSTRUCTION)

Not there yet.

## Help wanted

These are important issues where help is most appreciated:

* **Android support**: make Samizdat Node run on Android.
    * Why it matters: this is an end-user product and most end-users are on mobile.
    * Why it's hard: I'm bored by Android development. (Linux, macOS and Windows are
      already supported via `samizdat-up`.)

## Architecture

Hybrid peer-to-peer: nodes connect to hubs. Nodes produce, consume and transfer content. Hubs handle routing, discovery and NAT traversal. A node can connect to many hubs at once, so content drifts across tribes over time.

For the crate-by-crate tour, see [docs/architecture.md](docs/architecture.md).

## Installation

The recommended path is `samizdat-up`, which downloads the latest release from the
network itself and installs the node, hub or proxy as a system service. On Linux
and macOS:

```
curl -fsSL https://series-v5bknud2nujn5bmgrmtmxovrncwhedw4a6jtrnhz4yn3ovm2wxjq.hubfederation.com/latest/install.sh | sudo bash
sudo samizdat-up install node
```

On Windows, download `samizdat-up.exe` from the same location and run
`samizdat-up install node` from an elevated shell. See
[docs/operations.md](docs/operations.md) for the testbed runbook.

## Quick start

The installation ships the `samizdat` CLI. Run `samizdat init` in an empty
project directory: it creates a manifest (`Samizdat.toml`) and a private
manifest (`.Samizdat.priv`, secrets only -- add to `.gitignore`). The
private key is printed once on stdout; back it up.

`samizdat init` also registers a new _series_ with your node. A series has
a public key (what the network sees) and a node-local **nickname** (the
project directory name by default, overridable with `--nickname <x>`).
The nickname is a label your own node uses to find the series; nobody
else sees it.

To refresh the series, run `samizdat commit` (or `samizdat watch` for
refresh-on-save). It runs the build script in `Samizdat.toml` and
publishes the result.

**`samizdat commit` always publishes under your `[debug]` series, not your
public one.** This is on purpose: the public series is a sign-once-and-it's-
out-there operation. To push to the public series, pass `--release`.

After a `commit`, the content is reachable locally at the node's per-series
subdomain:

```
http://series-<base32-of-public-key>.localhost:4510/path/to/stuff
```

Each series gets its own browser origin, so storage, cookies and service
workers are isolated from every other series. `samizdat commit` prints
the URL; `samizdat series ls` has a `host_label` column with the same
string.

To share with friends, give them the public-key form on the public proxy:

```
https://series-<base32-public-key>.hubfederation.com/path/to/stuff
```

The proxy uses the same host-form as the node, so the leftmost label maps
to the same entity on both sides.

A friendlier subdomain form is also available via a blockchain identity
(Polygon, `samizdat identity create`): `http://<identity>.localhost:4510/`
locally and `https://<identity>.hubfederation.com/` via the proxy.
Registering an identity costs gas, and the name must be a valid DNS
label (`[a-z0-9-]`, 1-63 chars, no leading or trailing hyphen). Most
projects skip identities and share the public-key URL.

More under [docs/](docs/).


## Repository structure

* `common`: code shared across crates -- RPC definitions, Merkle tree, etc.
* `hub`: the Samizdat Hub crate.
* `node`: the Samizdat Node crate.
* `cli`: the Samizdat CLI crate.
* `proxy`: bridges a Samizdat Node to the open Web. Used by [https://hubfederation.com](https://hubfederation.com).
* `samizdat-up`: cross-platform installer and service manager (systemd, launchd, Windows SCM).
* `js`: the SamizdatJS library, so Web apps can talk to the local Samizdat node.
* `install`: installation artifacts for end users on different platforms.
* `simulate-net`: spawn your own network locally. Needed for integration tests.
* `blockchain`: smart contracts for the Samizdat identity.
* `terraform`: infrastructure-as-code for the public testbed.
* `docs`: architecture, threat model, conventions, operations runbook.

## Licensing

All code under the Samizdat Project is Free Software and is licensed to any individual or
    organization under the AGPLv3 license. You are free to run, study, alter and redistribute
    the software as you wish, as long as you abide by the terms of the aforementioned license.

Copyright 2021-2026 Tokahuke

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details. The text of this license
can be found in the [license](./license) file in this repository.
