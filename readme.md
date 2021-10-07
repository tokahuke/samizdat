# Samizdat: your content, available.

[![Continuous Integration](https://github.com/tokahuke/samizdat/actions/workflows/deploy-testbed.yaml/badge.svg?branch=stable)](https://github.com/tokahuke/samizdat/actions/workflows/deploy-testbed.yaml)

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

> How to make this warning disappear? Contribute! I am but one humble huma being.

## Project goals

Samizdat (from a Russian term meaning "self-publishing") aims to provide a decentralized internet application that enables one to do the following:

1. Be able to allow one to serve a public, static site without the need for a hosting service. The content is to be hosted in the person's own device or in caches from people who visit the site.

2. Provide a human-friendly identifier for resources contained in this network, i.e., a URL scheme. This URL is to be content-addressed, not location-addressed.

3. Oblivious hosting: only the device serving the content and the device asking for the content can extract any information about the content or its metadata.

4. Do all this _easily_ and _conveniently_. Graphical interfaces, mobile apps and amenities are welcome.

We are not quite there yet...

## Architecture

The project uses a hybrid peer-to-peer network, where nodes connect to hubs. The nodes are the consumers and producers of content; all content transmission is handled by the nodes. The hubs are used for signaling and NAT traversal. One node can connect to many hubs simultaneously so that content can diffuse through different tribes with time.

## Quick setup

### Linux

If you are interested in running this in your computer, you will need to build it from source, by now. An install script is provided to compile, install and enable the systemd service. Just run
```bash
./install.sh
```
This will spin up a server in `localhost:4510`, to where you can upload content using
```bash
curl -X POST http://localhost:4510/_object \
     -H "Content-Type: <your content type>" \
     --data-binary <your file>
```
Then, you can view it in your preferred browser in
```
http://localhost:4510/_objects/<the hash you received from CURL>
```

This link **can be copied and shared** just as if it were a true URL, because it actually is! Somebody running Samizdat on their computer will be able to see your file by accessing that same link.

To uninstall the node, a script is also provided:
```bash
./uninstall.sh
```

### MacOS

Should be similar to the above, no changes needed (although I have not tested).

### Windows

Some translation needed, but the compilation will work and produce a working binary. 

## Open issues

* Sending large files. By now, only one peer will send the whole files. On the bright side, Merkle Trees are already implemented; only parallelization is missing.
* Scalability:
    1. Hubs broadcast queries, in a slightly informed way, but we can do better.
    2. Clients are forced to do an `O(n)` search, instead of the typical `O(lg n)`. Blessing in disguise?
* Identities: you want nice personal handles, don't you? 
* Anti-censorship: it is hard for hubs to censor, but malicious nodes run by _them_ can exploit the system to
    1. Query if you have a copy of The Book.
    2. Serve you a copy of The Book and then send you to room 101.
* UX: do you have a freakin' mobile app?

## Licensing

Copyright 2021 Pedro B. Arruda

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details. The text of this license can be found in the [license](./license) file in this repository.
