# Browser security

What the browser guarantees for content served through samizdat, and
why those guarantees hold by construction rather than by samizdat-
specific defenses.

## TL;DR

Every entity samizdat serves (object, series, collection, edition,
identity) gets its own DNS label and therefore its own browser origin.
Storage scoping, cross-document access, cookie scope, service worker
scope, and CORS are then enforced by the browser between entities the
same way they are enforced between any two websites on the open web.
The samizdat-specific code path is "rewrite host, forward bytes."

## Trust model

The system trusts:

- **The user's browser.** Origin enforcement, mixed-content blocking,
  cookie scope, and the same-origin policy are doing the load-bearing
  work.
- **The user's OS.** Process isolation, file-system permissions on the
  node's data dir, launchd/systemd unit ownership.
- **The publisher of a series.** Whatever they put in their page runs
  in their entity's origin. This is the same trust the open web puts
  in a site author.

The system does NOT trust:

- **Other series operators on the same browser.** Series A cannot read
  series B's storage, cookies, or service-worker state.
- **Network observers** beyond what TLS hides. IP and timing leak.
- **Third-party hosts** that a publisher chooses to embed. Their hosts
  see what the publisher's page asks them to see.
- **The blockchain.** A typo in an on-chain identity registration
  stays there.

## The design decision

Each entity is its own browser origin via a typed-prefix DNS label
under the system's wildcard root:

| Pattern                        | Serves                          |
|--------------------------------|---------------------------------|
| `<root>`                       | proxy welcome / node admin      |
| `<identity>.<root>`            | identity content                |
| `series-<key>.<root>`          | series content (current edition)|
| `object-<hash>.<root>`         | raw object bytes                |
| `collection-<hash>.<root>`     | snapshot item lookup            |
| `edition-<id>.<root>`          | specific signed edition         |

Locally the same shape sits under `localhost:<port>`. The browser
treats each subdomain as a distinct site.

## Why this is safe

Each browser primitive that used to be a concern is now correct by
construction:

- **`localStorage`, `sessionStorage`, `IndexedDB`, Cache Storage.** All
  origin-scoped. Series A's pages cannot touch series B's data.
- **Cookies.** Origin-scoped. Samizdat publishers do not set
  `Domain=<root>`, so cookies stay at the entity origin.
- **Service workers.** Registration is origin-scoped. A SW from series
  A cannot intercept fetches for series B.
- **Cross-document DOM access.** Same-origin policy bars it.
- **`postMessage`.** Origin allowlists are explicit; no broadcast.
- **CORS.** The node's admin endpoints (`/_*` on bare loopback) are a
  distinct origin from any content page. A content page can only reach
  admin endpoints via an explicit OAuth-style consent grant (below).

## Administrative scope: consent UI

A content page can request administrative capabilities
(`ManageSeries`, `ManageBookmarks`, `ManageHubs`, ...) by opening
`/_register` on the bare loopback origin. The flow:

- The page lists the scopes it wants.
- The consent screen renders the requested scopes, explains the
  consequences of each, and disables the "Allow" button for ~3
  seconds so the user can read before clicking.
- A grant is per-origin and per-scope-set; revoke via the same
  surface.

Scopes are intentionally coarse. Granting `ManageSeries` to a page
gives it full series-management. The design choice is that capability
minimization happens at the consent screen, not at the API; a finer-
grained scoping scheme would just push the decision into the UI under
a longer scope list. The single line of defense here is the consent
screen itself.

A user who clicks Allow on a grant they should have denied is not
saved by any of the above. That is the limit of what this layer
defends.

## What this does NOT defend against

- **Publisher-supplied JavaScript on a page you choose to visit.** If
  you load `series-X.<root>`, the publisher of X controls everything
  that runs in their origin. Origin scoping protects every other
  series from X; it does not protect you from X.
- **Network-layer observers.** The proxy operator sees every request
  in cleartext past the TLS terminator. So does any third-party host
  the publisher's page embeds. Standard web caveats.
- **Third-party embeds.** A publisher who embeds Google Fonts is
  telling Google about every viewer. Samizdat does not strip these on
  the way out.
- **On-chain identity mistakes.** The blockchain is the blockchain.
- **A misclicked consent screen.** The 3-second delay slows the user
  down; it does not save a user determined to click through.

## What changed to get here

The earlier samizdat surface served every series under one path-form
prefix at one browser origin. In that world, samizdat had to write
substitutes for every browser primitive that should have done the
work: a KVStore so JS would not reach for the unscoped `localStorage`,
per-page CSP gymnastics, custom CSS-context isolation in a wrapped
template, a commit-time `~/` HTML rewriter so absolute paths would
resolve under the path prefix. Each of those was a defense against a
problem the browser would have solved for free if origins had been
distinct.

The per-entity-origin move did not add defenses. It removed the
constraint that was preventing the browser from defending. The
samizdat-specific substitutes got deleted along the way. The proxy
daemon is now a host-rewriting forwarder; the bytes the node returns
are the bytes the browser sees.
