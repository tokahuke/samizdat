# Proxy / local app-behavior divergence

## Scope and motivation

The proxy and the node both expose the same five-class typed-subdomain
surface (`object-<hash>.<root>`, `series-<key>.<root>`,
`collection-<hash>.<root>`, `edition-<id>.<root>`, `<identity>.<root>`).
Each entity owns a browser origin on both surfaces, so storage
partitioning, service-worker scope, same-origin reach, and relative-URL
resolution behave the same whether a viewer used `localhost:<port>` or
`proxy.hubfederation.com`. This file catalogues what still differs.

## A. Closed by the typed-subdomain refactor

The previous version of this document listed four reopened threats and
a class of absolute-path URL breakage. All are now closed:

- **T1 (service worker cross-series): CLOSED.** The proxy dispatcher
  routes by the same prefix-label scheme as the node, so a service
  worker registered under `series-A.<root>` cannot intercept fetches
  for `series-B.<root>`.
- **T9 (cross-series JS pull): CLOSED.** `<script src>` across entities
  is cross-origin on the proxy as it is on the node; the foreign body
  is opaque to the caller.
- **T10 (Cache / Cache-Storage cross-series fingerprint): CLOSED.**
  Cache Storage, `localStorage`, `sessionStorage`, IndexedDB, and
  cookies are partitioned per browser origin, and the proxy now serves
  each entity from a distinct origin.
- **T15 (identity vs path confusion): CLOSED.** `history.pushState`
  across entities is a cross-origin operation the browser refuses on
  the proxy as on the node.
- **Absolute-path URLs (`/style.css`, `fetch('/data.json')`,
  `<script src="/lib.js">`): CLOSED.** Each entity owns its origin
  root on both surfaces, so absolute paths resolve inside the calling
  entity without any proxy-side HTML rewrite.

The shared driver for those reopens (one external host fronting every
entity) is gone; the proxy now uses one wildcard cert and one wildcard
A record covering `*.<root>`, with the prefix label inside the single
wildcard component.

## B. What still differs

### B1. HTTPS termination and ACME

Only the proxy terminates TLS. The node speaks plain HTTP on loopback
and relies on the proxy (or the operator's own reverse proxy) for any
TLS-bearing surface. The proxy carries the wildcard ACME state machine,
DNS-01 plumbing (DigitalOcean, Cloudflare, Route53, or script-provider),
and the cert renewal lifecycle; the node has none of that. Operators
who run the node alone get no HTTPS.

### B2. Donation modal and Google Fonts

`proxy/src/html.rs::proxy_page` wraps upstream HTML in a donation-modal
shell (`proxy/templates/proxied-page.html.jinja`), which the node does
not do. The wrapper adds:

- a same-origin `<style>` and `<script>` block scoped by a random
  per-render namespace prefix;
- a `localStorage` counter (now per-entity-origin, since the entity
  owns its subdomain) that triggers the modal every N page views;
- two `<link rel="preconnect">` plus a Google Fonts stylesheet for
  "Poppins" and "Space Mono". Every proxied-page view emits requests
  to `fonts.googleapis.com` and `fonts.gstatic.com`, leaking the
  viewer's IP and `Referer` to Google. This is T17 in
  `docs/javascript-security.md`; it remains a proxy-only surface
  because the node does not serve the wrapper template.

`proxy_page` does NOT rewrite URLs in the upstream HTML. The selectors
in `proxy/src/html.rs` only splice `<head>` and `<body>` inner HTML
into the wrapper; no `<base>`, no anchor / src rewrite. Authors do not
need such a rewrite anymore because both surfaces use per-entity
origins and absolute paths resolve correctly on either.

### B3. Header forwarding allowlist

`PROXY_HEADERS` in `proxy/src/http.rs` lists the response headers the
proxy forwards from the node to the external client. The four
content-side security headers the node emits today
(`X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`,
and the admin-only `X-Frame-Options`) are forwarded; CSP, COOP, COEP,
CORP, `Strict-Transport-Security`, and `Clear-Site-Data` are not.
Any future node-side hardening that lands one of those without
updating `PROXY_HEADERS` will silently fail at the proxy. This is the
remaining T14 gap; tracked in `docs/javascript-security.md`.

### B4. Request-side stripping

The proxy strips `Authorization` and `Referer` on every forwarded
request and refuses non-GET methods, so every proxy-routed request
resolves as `entity = None` with `granted = [Public]` at the node.
Local browsing speaks directly to the node and carries the headers
through, so the trusted-context and bearer-token paths are reachable
locally and unreachable via the proxy. This is by design: admin
endpoints are not proxy-reachable, which is the load-bearing reason
most node admin routes are safe to expose at all.

## C. Conclusions

- Per-entity origin partitioning now applies on both surfaces; the
  T1/T9/T10/T15 reopens and the absolute-path breakage are gone.
- Remaining divergences are localized: TLS termination (B1), the
  donation-modal wrapper and its third-party font fetch (B2), the
  header forwarding allowlist (B3), and the request-side stripping
  that keeps admin endpoints out of proxy reach (B4).
- The largest residual is B2's Google Fonts leak: every proxy
  pageview phones home to Google with the viewer's IP. The fix is to
  self-host the fonts inside the proxy origin or drop the custom
  typography from the wrapper.
