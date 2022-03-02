# Samizdat: your content, available.

[![Continuous Integration](https://github.com/tokahuke/samizdat/actions/workflows/deploy-testbed.yaml/badge.svg?branch=stable)](https://github.com/tokahuke/samizdat/actions/workflows/deploy-testbed.yaml)
![Version 0.1 Codename Vavilov](https://img.shields.io/badge/version-0.1--vavilov-informational)

## Website

Samizdat is pulling itself by its bootstraps!
https://proxy.hubfederation.com/_series/fGfgc7ibvwy26U7nHjcaAhYmyLvXl84Ld-qab_0PPJc/

## Donate

If you support this work, consider donating using crypto

| Currency | Address                                      |
|----------|----------------------------------------------|
| `ETH`    | `0xba89B660eB6f5D894830C9273a5Dfb8dDc170cff` |
| `BTC`    | `bc1qseae89zr4z2lkl82nvvr6c9sl97agshapzeag5` |


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

* **Samizdat identity**: offer human-readable handles to entities inside the network.
  * Why it matters: do you browse the network using IP addresses or do you use DNS?
  * Why it's hard: this involves _consensus_ in the net. Yeah, Byzantine Generals, Bitcoin and stuff. 

* **Multi-platform support**: make Samizdat Node run on Mac, Windows and Android.
    * Why it matters: this is an end-user product and end-users are mostly trapped in these platforms.
    * Why it's hard: I'm too lazy to open my Windows 10, I don't have a Mac. 

## Architecture

The project uses a hybrid peer-to-peer network, where nodes connect to hubs. The nodes are the consumers and producers of content; all content transmission is handled by the nodes. The hubs are used for routing, discovery and NAT traversal. One node can connect to many hubs simultaneously so that content can diffuse through different tribes with time.

## Installation

Go [here](https://proxy.hubfederation.com/_series/fGfgc7ibvwy26U7nHjcaAhYmyLvXl84Ld-qab_0PPJc/install). This is a _proxy_ to the Samizdat Network which will allow you to download the latest version from the network itself.

## Quick start

In the installation, the `samizdat` cli tool is included. You can run `samizdat init` to create a new Samizdat project in your current directory. This will create a manifest file `Samizdat.toml` and a private manifest `.Samizdat.priv`, which will be added to your `.gitignore`. This file contains private credentials that you have to backup elsewhere dearly.

In your local hub, this will also create a new _series_, your very own microblog/directory in the Samizdat Network. To refresh the contents of your series, just do `samizdat commit`. Samizdat will run a build script that you supply in `Samizdat.toml`. Your content will be available in the URL:

```
http://localhost:4510/_series/<series key>/path/to/stuff
```

Despite the `localhost`, this is a public URL. You can share with your friends that have Samizdat installed that 
they will be abe to access it.

This is just the tip of the iceberg, however! Check out more [here](https://proxy.hubfederation.com/_series/fGfgc7ibvwy26U7nHjcaAhYmyLvXl84Ld-qab_0PPJc/docs).


## Licensing

All code under the Samizdat Project is Free Software and is licensed to any individual or
    organization under the AGPLv3 license. You are free to run, study, alter and redistribute
    the software as you wish, as long as you abide by the terms of the aforementioned license.

Copyright 2021 Tokahuke

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details. The text of this license
can be found in the [license](./license) file in this repository.
