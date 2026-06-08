# Proxy / local app-behavior divergence

## Scope and motivation

The node migration moved per-series content onto subdomain origins
(`<base32-key>.localhost:4510`, `<identity>.localhost:4510`); the public
proxy at `proxy.hubfederation.com` kept its path-shaped external surface
(`/_series/<base64-key>/<rest>`, `/~<identity>/<rest>`) and rewrites
each incoming request internally into the node's host-form upstream
(`proxy/src/http.rs:83-99`, `proxy/src/http.rs:164-210`). This means a
single JS app authored as a Samizdat series sees two very different
browser-origin shapes depending on which surface a viewer used, and any
property that follows the browser origin (storage partitioning, service
worker scope, same-origin reach, relative URL resolution) diverges. This
report audits that divergence.

## A. Threats reopened at the proxy origin

The per-series subdomain dispatcher
(`node/src/http/host_scope.rs:96-140`) gives each series and each
identity its own browser origin locally. Through the proxy, every
series and every identity is served under one origin,
`proxy.hubfederation.com`, because the path-form is preserved on the
external surface (`proxy/src/http.rs:60-99`) and only the upstream URL
to the node is host-rewritten (`proxy/src/http.rs:164-210`). The
browser-origin partitioning that closed T1, T9, T10, T15 locally is
therefore absent on the proxy side.

- **T1 (service worker cross-series): REOPENED at the proxy.** A page
  served at `proxy.hubfederation.com/~A/...` can register a service
  worker (`navigator.serviceWorker.register('/sw.js')`); the registered
  scope is rooted at `proxy.hubfederation.com/` (or at whatever path
  the script and its `Service-Worker-Allowed` header permit). The
  worker intercepts every subsequent fetch on that origin, which
  through the proxy means every other series and identity the user
  visits. Driver: the proxy keeps a single external host
  (`proxy/src/http.rs:33-38`, routes `/{*path}` and `/`); the upstream
  per-series host rewrite happens only on the server-to-node hop
  (`proxy/src/http.rs:182-206`) and is invisible to the browser. The
  local mitigation cited in `docs/javascript-security.md:98-115` does
  not apply.
- **T9 (cross-series JS pull): REOPENED at the proxy.** On the local
  node, `<script src="http://<other-key>.localhost:4510/lib.js">` was
  a cross-origin fetch and the foreign script body was opaque to the
  caller. On the proxy, the equivalent reference is
  `<script src="/_series/<other-key>/lib.js">` or
  `<script src="/~other/lib.js">`, which is SAME-origin under
  `proxy.hubfederation.com`. The fetched body is fully readable to the
  importing page's JS (the network response is same-origin), and once
  executed it runs in the importer's context with full DOM and storage
  access for that single shared origin. Driver: same as T1, the proxy
  uses one external host (`proxy/src/http.rs:33-38`).
- **T10 (Cache and Cache-Storage as cross-series fingerprint):
  REOPENED at the proxy.** `localStorage`, `sessionStorage`, Cache
  Storage, IndexedDB, cookies, BroadcastChannel are partitioned per
  browser origin. With a single origin
  (`proxy.hubfederation.com`) for all series and all identities, a
  marker written by series A is readable by series B with no
  cooperation, defeating the partitioning model
  `docs/javascript-security.md:339-357` relied on. Same driver
  (`proxy/src/http.rs:33-38`). The template itself relies on the
  shared `localStorage` it has access to under this single origin
  (`proxy/templates/proxied-page.html.jinja:146-159`, key
  `__samizdat_proxy_page_count`), which is itself evidence that the
  proxy origin is shared across series; an attacker can read or
  overwrite that same key.
- **T15 (identity vs path confusion): REOPENED at the proxy.** The
  local mitigation (`docs/javascript-security.md:502-510`) was that
  `history.pushState` could not navigate to a different identity's
  URL because that would require crossing the subdomain origin. On
  the proxy everything is path-shaped under one origin, so a page at
  `proxy.hubfederation.com/~attacker/...` can call
  `history.pushState({}, '', '/~bank.example/login')` and the address
  bar will show `proxy.hubfederation.com/~bank.example/login` while
  the document and JS context remain the attacker's. Driver:
  `proxy/src/http.rs:33-38` plus the path-form preservation in
  `do_proxy` (`proxy/src/http.rs:60-99`).

Beyond T1/T9/T10/T15 the asymmetry introduces or worsens two further
issues:

- **Proxy template injects a same-origin `localStorage` key
  unconditionally.** `proxy/templates/proxied-page.html.jinja:146-159`
  writes the integer counter `__samizdat_proxy_page_count` into the
  shared `localStorage`. Because every series is the same origin
  through the proxy, this counter is *shared across all series viewed
  through the proxy*, and it is writable by any series' JS at that
  origin. That is both a cross-series tracking signal (the counter
  increments persistently and is visible to every site) and a
  defacement primitive (a series can clobber the key to suppress the
  donation modal forever on a viewer's browser or, conversely,
  fabricate a visit count to make the modal appear for every page
  load). Neither is "security catastrophic" but both are
  proxy-specific and worth naming.
- **`proxy_page` does NOT introduce a forced same-origin embedding of
  series B inside series A.** I read `proxy/src/html.rs:25-54` in
  full; the rewrite only picks `<head>` and `<body>` inner HTML via
  the `head` and `body` selectors (`proxy/src/html.rs:8-11`) and
  splices them into the donation-modal template
  (`proxy/templates/proxied-page.html.jinja`). It does not inject
  `<base>`, does not inject `<iframe>` cross-series, and does not
  parse or rewrite `<script src>` / `<link href>` / anchor targets in
  the embedded body. The same-origin reach that series A's JS gains
  over series B is purely a consequence of the proxy serving them
  under one external host, not of any HTML rewrite by `proxy_page`.
  So the answer to the explicit "does `proxy_page` create a
  same-origin trust relationship between A and B that A's author did
  not consent to?" question is: no, not via HTML rewrite. It does so
  via the *origin policy* of the proxy as a whole, which is the
  reopening of T1/T9/T10 above.

## B. Author-facing annoyances

The proxy preserves the path-form URL on the external surface
(`/_series/<base64-key>/<rest>` and `/~<identity>/<rest>`,
`proxy/src/http.rs:60-77`). The HTML rewrite does NOT touch
URL-bearing attributes (`proxy/src/html.rs:25-54` only splits head
and body); there is no `<base>` injection and no anchor / src
rewriting. The consequences:

- `<a href="/style.css">` (absolute path).
  - Local origin `<base32-key>.localhost:4510`: resolves to
    `<base32-key>.localhost:4510/style.css`. Correct, hits the same
    series' root.
  - Proxy origin `proxy.hubfederation.com`: resolves to
    `proxy.hubfederation.com/style.css`. WRONG: this hits the proxy
    handler at the root (`proxy/src/http.rs:33-38`), which parses
    `/style.css` as `entity = "_identity"`, `content_hash = "style.css"`
    (`proxy/src/http.rs:71-77`), i.e. an identity lookup for the handle
    `style.css`. That handle will fail `check_servable_identity`
    (literal dot in the handle) and the proxy will return 400 with a
    body suggesting `_series/<base64-key>/` form
    (`proxy/src/http.rs:85-98`). The CSS does not load. No proxy
    rewrite compensates.
- `<a href="style.css">` (document-relative).
  - Local: resolves under the current path in
    `<base32-key>.localhost:4510/...`. Works.
  - Proxy: resolves under the current path
    `proxy.hubfederation.com/_series/<base64-key>/<dir>/style.css`.
    Works, because the path prefix is preserved on the external
    surface and the proxy routes it correctly. Relative paths are
    the only URL shape that survives both surfaces.
- `<a href="../other-page/">` (parent-relative).
  - Local: walks up within `<base32-key>.localhost:4510/...`. Works
    until it tries to escape the series root, at which point it lands
    on the bare loopback (admin) origin, which is a different origin.
  - Proxy: walks up within
    `proxy.hubfederation.com/_series/<base64-key>/<dir>/`. Works
    within the series; escaping past `/_series/<base64-key>/` lands
    on the proxy root with the parsing in
    `proxy/src/http.rs:71-77`, which will be a 400 or a wrong
    identity lookup. Mostly fine in practice as long as authors do
    not walk above the series root.
- `<a href="http://<other-key>.localhost:4510/foo">` (cross-series
  absolute, hard-coded local host).
  - Local: works.
  - Proxy: the browser tries to resolve `<other-key>.localhost`
    against public DNS and fails. The proxy does not rewrite this
    (`proxy/src/html.rs:25-54` does not touch anchors or `src`
    attributes). Broken link.
- `<a href="https://proxy.hubfederation.com/_series/<other-key>/foo">`
  (cross-series absolute, proxy form, hard-coded).
  - Local: external link to the proxy; opens in the public proxy.
    Works but takes the user off the local origin.
  - Proxy: stays on `proxy.hubfederation.com`. Works as a navigation;
    same-origin implications: every linked-to series shares storage,
    cookies, and service-worker scope with the linker. This is the
    T1/T9/T10 reopening in concrete form.
- `fetch('/data.json')`.
  - Local: hits `<base32-key>.localhost:4510/data.json`, served by
    the series. Works.
  - Proxy: hits `proxy.hubfederation.com/data.json`, routed by
    `proxy/src/http.rs:60-77` as identity `data.json` and rejected
    with 400. The fetch fails. No rewrite compensates.
- `window.location.origin` and `document.domain`.
  - Local: `http://<base32-key>.localhost:4510` (or
    `<identity>.localhost`). Different for every series.
  - Proxy: `https://proxy.hubfederation.com` for every page.
  - Authors who store `localStorage` keyed by `location.origin`
    accidentally key globally on the proxy.
  - Authors who use `location.pathname` for routing must allow for
    the `/_series/<base64-key>/` or `/~<identity>/` prefix when
    served via proxy; locally they do not see that prefix.

The "broken absolute path" case (`/style.css`, `fetch('/data.json')`,
`<script src="/lib.js">`) is the worst day-one annoyance: it is the
single most common URL shape, and it silently fails on the proxy with
no rewrite to save it. The local origin makes absolute paths
"just work"; the proxy origin breaks them because the path namespace
is shared with the `/_series/...` and `/~...` dispatch prefixes.

## C. What `proxy_page` actually does

`proxy_page` (`proxy/src/html.rs:25-54`) is intentionally minimal:

- Parses the upstream HTML with `scraper::Html::parse_document`
  (`proxy/src/html.rs:27`).
- Picks the inner HTML of the first `<head>` element with the `head`
  selector (`proxy/src/html.rs:8-9`, `proxy/src/html.rs:28-32`).
- Picks the inner HTML of the first `<body>` element with the `body`
  selector (`proxy/src/html.rs:10-11`, `proxy/src/html.rs:33-37`).
- Generates a random 32-bit CSS namespace prefix
  `samizdat_<hex>` to scope the donation-modal CSS / IDs
  (`proxy/src/html.rs:38-42`).
- Renders the askama template `proxied-page.html.jinja`
  (`proxy/src/html.rs:16-23`, `proxy/src/html.rs:44-53`) with: the
  upstream `<head>` inner HTML, the upstream `<body>` inner HTML, the
  random namespace, a hard-coded download link
  (`SAMIZDAT_BLOG_PATH = "/~samizdat/install/"`,
  `proxy/src/html.rs:13`), and `show_modal_every`
  (`proxy/src/html.rs:49`) from the proxy CLI.

The template (`proxy/templates/proxied-page.html.jinja:1-163`) wraps
the upstream content in a fresh `<html><head>...</head><body>...</body></html>`
shell, additionally injects:

- A `<meta charset="UTF-8">`
  (`proxy/templates/proxied-page.html.jinja:4`).
- Two `<link rel="preconnect">` plus a Google Fonts stylesheet `<link
  rel="stylesheet">` for "Poppins" and "Space Mono"
  (`proxy/templates/proxied-page.html.jinja:8-13`). This is the only
  third-party network reference the proxy itself adds; every viewer
  of any proxied page emits a request to `fonts.googleapis.com` and
  `fonts.gstatic.com`.
- A `<style>` block scoped by the random namespace
  (`proxy/templates/proxied-page.html.jinja:14-106`) and a donation
  modal `<div>` plus `<script>`
  (`proxy/templates/proxied-page.html.jinja:111-160`).
- A `localStorage` counter `__samizdat_proxy_page_count` that
  triggers the modal every `show_modal_every` page views
  (`proxy/templates/proxied-page.html.jinja:146-159`).

Selectors touched: ONLY `head` and `body` (top-level elements).
No selector for `<a>`, `<link>`, `<script>`, `<img>`, `<base>`,
`<form>`, `<iframe>`, or any Samizdat-specific class / attribute.
There is no Samizdat-specific markup handling. The upstream `<head>`
and `<body>` inner HTML is spliced verbatim into the wrapper template
via askama's `|safe` filter
(`proxy/templates/proxied-page.html.jinja:6`,
`proxy/templates/proxied-page.html.jinja:109`), trusting that the
upstream node already serves what the publisher intended.

Implication: the proxy does NOT rewrite any URL the publisher
authored. There is no `<base>` tag injection to retarget relative
links. There is no compensating rewrite for absolute-path links.
There is no rewrite to translate `<base32-key>.localhost:4510`
references into proxy-form. There is no scrub of inline
`<script src>` or `<link href>`.

## D. Security-headers asymmetry

The node sets the following on responses
(`node/src/http/mod.rs:188-258`):

- Global to admin + content: `X-Content-Type-Options: nosniff`
  (`node/src/http/mod.rs:243-246`), `Referrer-Policy: same-origin`
  if not present (`node/src/http/mod.rs:247-250`),
  `Permissions-Policy: interest-cohort=()` if not present
  (`node/src/http/mod.rs:251-254`).
- Admin only: `X-Frame-Options: DENY`
  (`node/src/http/mod.rs:188-192`) and a strict
  `Content-Security-Policy`
  (`node/src/http/mod.rs:193-205`). These reach the bare loopback
  admin origin only; the proxy never serves admin paths
  (`node/src/http/mod.rs:293-315`'s `require_bare_host` 404s admin
  requests on `*.localhost`, which is what the proxy's upstream URL
  uses).

`PROXY_HEADERS` (`proxy/src/http.rs:16-31`) controls which response
headers the proxy forwards from the node to the external client.
Walking the allowlist:

Forwarded to the browser:

- `ETag` (`proxy/src/http.rs:17`). Caching identity, not a security
  header.
- `X-Samizdat-Bookmark`, `X-Samizdat-Object`, `X-Samizdat-Is-Draft`,
  `X-Samizdat-Collection`, `X-Samizdat-Series`,
  `X-Samizdat-Edition`, `X-Samizdat-Query-Duration`
  (`proxy/src/http.rs:18-24`). Samizdat metadata.
- `X-Content-Type-Options` (`proxy/src/http.rs:27`). Forwarded.
- `X-Frame-Options` (`proxy/src/http.rs:28`). Forwarded. Note: the
  node only sets this on admin, and the proxy cannot serve admin, so
  in practice no content response carries this header from the node.
  The header is allowed through if upstream ever starts to send one
  for content.
- `Referrer-Policy` (`proxy/src/http.rs:29`). Forwarded.
- `Permissions-Policy` (`proxy/src/http.rs:30`). Forwarded.

Additionally set by the proxy on every response:

- `Content-Type` (`proxy/src/http.rs:114-121`). Copied through; if
  upstream sets none, the proxy defaults to `text/plain`.

NOT forwarded (silently dropped if upstream sends them):

- `Content-Security-Policy`. Not in `PROXY_HEADERS`. Today the node
  only sets CSP on admin responses, which the proxy cannot reach, so
  the present-day yield is zero. If the node ever adds a CSP for
  content responses (the deferred T4 / T9 / T11 work in
  `docs/javascript-security.md:519-528`), the proxy will strip it and
  the proxy-viewer audience will be unprotected.
- `Cross-Origin-Opener-Policy`, `Cross-Origin-Embedder-Policy`,
  `Cross-Origin-Resource-Policy`. Not in the allowlist; same future-
  regression risk.
- `Strict-Transport-Security`. The proxy serves over TLS itself; if
  the operator wants HSTS on the proxy origin they have to add it
  outside this list. Not present in `PROXY_HEADERS`.
- `Clear-Site-Data`. Not in the allowlist. If the node adds an
  unsubscribe-clears-storage hook (the T10 mitigation idea in
  `docs/javascript-security.md:354-357`), the proxy will not pass it
  through.

Browser-facing result of the asymmetry:

- The four security headers the node currently emits on content
  (`X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`)
  plus the admin-only `X-Frame-Options` all reach the proxy viewer
  by virtue of `proxy/src/http.rs:27-30`. The "node-side hardening
  silently bypassed by the proxy" warning from
  `docs/javascript-security.md:451-465` (T14) is partially addressed:
  the security-headers allowlist now includes the four that exist.
- CSP and the COOP/COEP/CORP trio are still NOT in the allowlist.
  Any future node-side hardening that lands those without updating
  `PROXY_HEADERS` will silently fail at the proxy. This is the
  remaining T14 gap.

## E. Conclusions

- The proxy serves all series and all identities at one origin
  (`proxy.hubfederation.com`); the per-series subdomain isolation
  that closed T1, T9, T10, T15 locally does NOT apply through the
  proxy. All four are REOPENED for any viewer using the proxy.
- The single worst app-portability gotcha is absolute-path URLs.
  Locally, `<link href="/style.css">`, `<script src="/lib.js">`,
  and `fetch('/data.json')` all just work because the series owns
  its own origin root. Through the proxy they resolve to
  `proxy.hubfederation.com/<name>`, where the path namespace is
  owned by the proxy router, and either 400 or hit the wrong
  identity. There is no rewrite in `proxy_page` to compensate.
  Authors who want their app to work in both surfaces must use
  relative paths only.
- `proxy_page` only splices upstream `<head>` and `<body>` into a
  donation-modal wrapper template
  (`proxy/src/html.rs:25-54`,
  `proxy/templates/proxied-page.html.jinja:1-163`); it does NOT
  rewrite URLs and does NOT introduce any cross-series same-origin
  trust beyond what the single-host external surface already does.
  The shared-origin problem is the proxy itself, not the rewrite.
- The proxy template writes a shared-origin `localStorage` key
  (`__samizdat_proxy_page_count`,
  `proxy/templates/proxied-page.html.jinja:146-159`) that is
  readable and writable by any series viewed through the proxy.
  This is a small cross-series signal in its own right and a
  defacement / suppression target for the donation modal.
- The security-headers allowlist (`proxy/src/http.rs:16-31`) now
  forwards the four headers the node emits on content
  (`X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`,
  plus admin-only `X-Frame-Options`). CSP, COOP, COEP, CORP,
  `Strict-Transport-Security`, and `Clear-Site-Data` are NOT in the
  allowlist; any future node-side hardening that lands those is
  silently stripped at the proxy.
- The proxy template unconditionally pulls Google Fonts
  (`proxy/templates/proxied-page.html.jinja:8-13`); the donation
  modal therefore leaks every proxied-page viewer's IP and request
  pattern to `fonts.googleapis.com` and `fonts.gstatic.com`. I could
  not verify in this audit whether that is the only third-party
  reference in the template chain; reader should grep before
  relying on it.
- I could not verify in this audit whether any node-side route
  inserts headers between the `global_layers` chain and the proxy's
  upstream HTTP call, e.g. via per-handler `SetResponseHeaderLayer`
  or per-route insert; reader should grep `node/src/http/` for
  `SetResponseHeaderLayer` and `insert_header` before assuming the
  forwarded-header set is exactly the four listed above.
