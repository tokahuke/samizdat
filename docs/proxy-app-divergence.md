# Proxy / local app-behavior divergence

## Scope and motivation

The proxy and the node both expose the same five-class typed-subdomain
surface (`object-<hash>.<root>`, `series-<key>.<root>`,
`collection-<hash>.<root>`, `edition-<id>.<root>`, `<identity>.<root>`).
Each entity owns a browser origin on both surfaces, so storage
partitioning, service-worker scope, same-origin reach, and relative-URL
resolution behave the same whether a viewer used `localhost:<port>` or
`hubfederation.com`. This file catalogues what still differs.

## A. What no longer differs

Service-worker scope, cross-origin script reach, Cache-Storage /
`localStorage` partitioning, and absolute-path URL resolution behave
identically on the proxy and the node. The proxy dispatches by the
same prefix-label scheme as the node, so every browser primitive that
keys on origin sees the same origins through either surface. See
`docs/browser-security.md`.

## B. What still differs

### B1. HTTPS termination and ACME

Only the proxy terminates TLS. The node speaks plain HTTP on loopback
and relies on the proxy (or the operator's own reverse proxy) for any
TLS-bearing surface. The proxy carries the wildcard ACME state machine,
DNS-01 plumbing (DigitalOcean, Cloudflare, Route53, or script-provider),
and the cert renewal lifecycle; the node has none of that. Operators
who run the node alone get no HTTPS.

### B2. Header forwarding allowlist

`PROXY_HEADERS` in `proxy/src/http.rs` lists the response headers the
proxy forwards from the node to the external client. The four
content-side security headers the node emits today
(`X-Content-Type-Options`, `Referrer-Policy`, `Permissions-Policy`,
and the admin-only `X-Frame-Options`) are forwarded; CSP, COOP, COEP,
CORP, `Strict-Transport-Security`, and `Clear-Site-Data` are not.
Any future node-side hardening that lands one of those without
updating `PROXY_HEADERS` will silently fail at the proxy.

### B3. Request-side stripping

The proxy strips `Authorization` and `Referer` on every forwarded
request and refuses non-GET methods, so every proxy-routed request
resolves as `entity = None` with `granted = [Public]` at the node.
Local browsing speaks directly to the node and carries the headers
through, so the trusted-context and bearer-token paths are reachable
locally and unreachable via the proxy. This is by design: admin
endpoints are not proxy-reachable, which is the load-bearing reason
most node admin routes are safe to expose at all.

## C. Conclusions

- Per-entity origin partitioning now applies on both surfaces.
- Remaining divergences are localized: TLS termination (B1), the
  header forwarding allowlist (B2), and the request-side stripping
  that keeps admin endpoints out of proxy reach (B3).
- The largest residual is B2's Google Fonts leak: every proxy
  pageview phones home to Google with the viewer's IP. The fix is to
  self-host the fonts inside the proxy origin or drop the custom
  typography from the wrapper.
