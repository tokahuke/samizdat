# Samizdat: a web of content

## Introduction

In these troubling times, some people might find it hard to publish content to the web. Samizdat is a P2P network for sharing and publishing content without the need of a server, most of which are run by _them_. Self-publish your content today with Samizdat!

### Warning

This is only a proof of concept implementation. It might work in a small scenario, but breaking changes will be needed to be made for it to scale for the whole Web.

And, as always, be aware that, since this is a nascent project, vulnerabilities might exist that nobody has any idea of yet. By now, tread carefully.

## Quick setup


## Open issues

* Sending large files. By now, an arbitrary 64MB limit imposed. Use chunks and Merkle Trees, like torrent.
* Scalability:
    1. Hubs broadcast queries.
    2. Clients are forced to do an `O(n)` search, instead of the typical `O(lg n)`. Blessing in disguise?
* Identities: how to know it was Goldstein who really wrote The Book?
* Incentives: how to make it profitable for people to run large nodes and hubs?
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
