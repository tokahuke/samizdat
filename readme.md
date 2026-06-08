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

In these troubling times, some people might find it hard to publish content to the web. Samizdat is a P2P network for sharing and publishing content without the need of a server, most of which are run by _them_. Self-publish your content today with Samizdat!

### Warning

This is still a proof of concept implementation. So three caveats are in place:

1. Don't rely on the availability of the network or of your content; have alternatives in place.
2. Expect frequent breaking changes.
3. Expect vulnerabilities. Do not use the network for sensitive content yet.

> How to make this warning disappear? Contribute! I am but one humble human being.

## Project goals

Samizdat (from a Russian term meaning "self-publishing") aims to provide a decentralized internet application that enables one to do the following:

1. Be able to allow one to serve a public, static site without the need for a hosting service. The content is to be hosted in the person's own device or in caches from people who visit the site. (READY)

2. Provide a human-friendly identifier for resources contained in this network, i.e., a URL scheme. This URL is to be content-addressed, not location-addressed. (IN CONSTRUCTION)

3. Oblivious hosting: only the device serving the content and the device asking for the content can extract any information about the content or its metadata. (BY DESIGN)

4. Do all this _easily_ and _conveniently_. Graphical interfaces, mobile apps and amenities are welcome. (IN CONSTRUCTION)

We are not quite there yet...

## 📢 Help wanted! 🗯

These are important issues where help is most appreciated:

* **Android support**: make Samizdat Node run on Android.
    * Why it matters: this is an end-user product and most end-users are on mobile.
    * Why it's hard: I'm bored by Android development. (Linux, macOS and Windows are
      already supported via `samizdat-up`.)

## Architecture

The project uses a hybrid peer-to-peer network, where nodes connect to hubs. The nodes are the consumers and producers of content; all content transmission is handled by the nodes. The hubs are used for routing, discovery and NAT traversal. One node can connect to many hubs simultaneously so that content can diffuse through different tribes with time.

For a deeper tour of the crates and concepts, see [docs/architecture.md](docs/architecture.md).

## Installation

The recommended path is the `samizdat-up` bootstrap installer, which downloads the
latest release from the network itself and then installs the node, hub or proxy as
a system service. On Linux and macOS:

```
curl -fsSL https://series-v5bknud2nujn5bmgrmtmxovrncwhedw4a6jtrnhz4yn3ovm2wxjq.hubfederation.com/latest/install.sh | sudo bash
sudo samizdat-up install node
```

On Windows, download `samizdat-up.exe` from the same location and run
`samizdat-up install node` from an elevated shell. See
[docs/operations.md](docs/operations.md) for the testbed runbook.

## Quick start

In the installation, the `samizdat` cli tool is included. Run `samizdat init`
in an empty project directory and it will create a manifest (`Samizdat.toml`)
and a private manifest (`.Samizdat.priv`, secrets only -- add to `.gitignore`).
You will be shown the private key once on stdout; back it up.

`samizdat init` also registers a new _series_ with your node. A series has a
public key (which is what the network sees) and a node-local **nickname**
(taken from the project directory name by default, overridable with
`--nickname <x>`). The nickname is a label your own node uses to find the
series; it carries no meaning to anyone else.

To refresh the contents of your series, run `samizdat commit` (or
`samizdat watch` for continuous refresh-on-save). It runs the build script
declared in `Samizdat.toml` and publishes the result.

**`samizdat commit` always publishes under your `[debug]` series, not your
public one.** This is on purpose: the public series is a sign-once-and-it's-
out-there operation. To push to the public series, pass `--release`.

After a `commit`, the content is reachable locally at the node's per-series
subdomain:

```
http://series-<base32-of-public-key>.localhost:4510/path/to/stuff
```

Each series lives at its own browser origin, so storage, cookies, and
service workers are isolated from every other series. The `samizdat commit`
output prints the URL for you; `samizdat series ls` also includes a
`host_label` column with the same string.

To share with friends, give them the public-key form on the public proxy:

```
https://series-<base32-public-key>.hubfederation.com/path/to/stuff
```

The proxy uses the same host-form as the node, so the leftmost label maps
to the same entity on both sides.

Samizdat also supports a friendlier subdomain form using a blockchain
identity (Polygon, via `samizdat identity create`), reachable at
`http://<identity>.localhost:4510/` locally and
`https://<identity>.hubfederation.com/` via the proxy. Registering
an identity costs gas and the name has to be a valid DNS label
(`[a-z0-9-]`, 1-63 chars, no leading or trailing hyphen). Most projects
skip identities and just share the public-key URL.

This is just the tip of the iceberg, however! Check out more under
[docs/](docs/) in this repository.


## Repository structure

* `common`: Rust lib defining common code shared by other Samizdat crates. You will find here RPC definitions, Merkle tree implementation, etc...
* `hub`: the Samizdat Hub crate.
* `node`: the Samizdat Node crate.
* `cli`: the Samizdat CLI crate.
* `proxy`: a proxy to bridge a Samizdat Node to the open Web, used in [https://hubfederation.com](https://hubfederation.com).
* `samizdat-up`: cross-platform installer / service manager (systemd, launchd, Windows SCM).
* `js`: the SamizdatJS library, which enables Web applications to interface with the local Samizdat node.
* `install`: installation artifacts for end users on different platforms.
* `simulate-net`: spawn your own network locally. Necessary for integration tests.
* `blockchain`: smart contracts for the Samizdat identity.
* `terraform`: infrastructure-as-code for the public testbed.
* `docs`: architecture, threat model, conventions and operations runbook.

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
