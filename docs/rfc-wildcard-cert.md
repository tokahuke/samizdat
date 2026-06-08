# RFC: wildcard TLS for the public proxy

## Status

Draft. Author: leave the byline empty. Targeted release: post-0.3.x.

## Motivation

`docs/proxy-app-divergence.md` catalogues four threats that reopen at the
proxy origin because every series is served under one host
(`proxy.hubfederation.com`). The structural fix is to serve each series under
its own subdomain (`<base32-key>.proxy.<domain>` and
`<identity>.proxy.<domain>`), mirroring what `node/src/http/host_scope.rs`
already does on the local node. That requires TLS certs valid for those
subdomains plus DNS that resolves them.

Constraint: an operator must be able to roll this out themselves with the
minimum possible manual configuration. The earlier draft of this RFC
proposed an ACME wildcard cert via DNS-01 challenge through an acme-dns
shim. That plan required three operator-side steps (run acme-dns, set a
CNAME in the real DNS, paste credentials into proxy config), each of which
is a potential drop-off point. Below is a simpler plan that needs ONE
operator step.

## Plan: on-demand HTTP-01 per subdomain

Use ACME HTTP-01 with on-demand cert provisioning per SNI. This is the
"automatic HTTPS" pattern that Caddy popularized.

### Operator-facing flow

1. Operator sets one wildcard DNS record:
   `*.proxy.<their-domain>` -> proxy host IP (A/AAAA).
2. Operator starts the proxy. That is the whole setup.

### Runtime flow

1. A request arrives at the proxy on port 443. The TLS layer reads the SNI
   (`<base32-key>.proxy.<domain>`).
2. If a cert for that SNI is in the on-disk cache, serve it.
3. If not, kick off an ACME HTTP-01 flow against Let's Encrypt for that
   exact SNI. The challenge URL
   `http://<base32-key>.proxy.<domain>/.well-known/acme-challenge/...` is
   reachable because of the wildcard A record from step 1 of the
   operator-facing flow; the proxy answers on port 80 from the same
   process. Once the cert is issued (typically one to two seconds), cache
   it and complete the TLS handshake.
4. Renewals run on schedule, per cert, in a background task.

The operator's DNS provider is irrelevant beyond setting the one wildcard
record. There is no CNAME, no acme-dns, no third-party credential, no
DNS-01 ceremony.

### What this requires of the proxy code

- A new `proxy/src/sni_acme.rs` (working name) that owns the cert cache
  and the on-demand ACME state machine. Layers on top of `rustls`'s
  `ResolvesServerCert` trait so the per-SNI lookup hooks into the TLS
  handshake.
- An ACME client capable of HTTP-01. Recommend `instant-acme`
  (https://crates.io/crates/instant-acme) which exposes a clean async
  API and lets the caller drive each step (account creation, order,
  authorize, finalise). The proxy hands `instant-acme` its own HTTP-01
  responder (just an in-memory map keyed by challenge token).
- A path on the existing axum router at
  `/.well-known/acme-challenge/{token}` that serves the responder's map.
  This already exists for the non-wildcard path; reuse it.
- The existing `rustls-acme` cert for the bare `proxy.<domain>` is kept
  by registering that name in the same on-demand flow on first hit; one
  code path.

### Cert cache shape

`proxy/<acme-cache>/certs/<sni>/{cert.pem, key.pem, meta.json}` per
subdomain. `meta.json` records the issue and expiry timestamps so the
renewal task can prioritise. Atomic write on swap (rename-over) so a
half-renewed cert never serves.

### Renewal

A background task wakes every hour, scans the cache for any cert within
30 days of expiry, renews via the same on-demand flow. Per-cert backoff
on failure; alarm in `samizdat doctor` (see open question 4 below) if a
cert is within 7 days of expiry and renewals keep failing.

## Costs and caveats

- **Cold-start latency.** The first request for a never-before-seen
  subdomain incurs one to two seconds while ACME negotiates. Subsequent
  requests reuse the cache. Acceptable for content browsing.
- **Let's Encrypt rate limit.** Currently 50 certs per registered domain
  per week (the entire `proxy.<domain>` zone counts as one registered
  domain for this limit). Documented enough for a personal proxy hosting
  a single-digit number of series owners. Document the ceiling in
  `docs/operations.md`. High-volume operators can opt into wildcard
  DNS-01 via acme-dns as a v2 path; the proxy's plumbing can take either.
- **Port 80 must be reachable.** HTTP-01 challenges require it. Same
  requirement as the existing non-wildcard cert; no change.
- **First request per SNI is sync-ish.** While ACME is in flight, the
  TLS handshake stalls. Should be safe given the latency budget;
  document that an operator behind a CDN may need to disable smart
  caching for `*.proxy.<domain>` requests during cert provisioning.

## Migration

The current proxy uses `rustls-acme` for one fixed domain. Migration:

1. Add `instant-acme` to `proxy/Cargo.toml`. Keep `rustls-acme` for now;
   the on-demand layer will subsume it.
2. Build the `sni_acme` module behind a `[wildcard] enable = true`
   config flag in `proxy.toml`. Default off; the testbed keeps working
   on the rustls-acme path until the operator flips the flag.
3. Once the on-demand path is proven on the testbed, drop the
   `rustls-acme` dependency and the legacy path. One ACME flow, one
   library, one config story.

No data migration, no client-visible URL change beyond the eventual 301
from path-form to host-form (see "URL surface" below).

## URL surface

`translate_to_node_url` in `proxy/src/http.rs` already rewrites the
external path-form to the upstream host-form against the local node. The
external surface change once the wildcard cert is in place:

- `proxy.<domain>/` continues to serve the welcome / docs path.
- `proxy.<domain>/_series/<base64>/<rest>` -> 301 to
  `https://<base32>.proxy.<domain>/<rest>`.
- `proxy.<domain>/~<identity>/<rest>` -> 301 to
  `https://<identity>.proxy.<domain>/<rest>`.
- Host-form requests serve content directly; `translate_to_node_url`
  becomes a no-op rewrite for them.

The 301s ensure pre-existing shared links keep working and the user's
browser lands on the per-series origin. T1/T9/T10/T15 reopens close
because each series is its own browser origin on the proxy too.

## Open questions

1. **Drop `rustls-acme` outright after on-demand lands, or keep both?**
   Recommendation: drop. One library, one flow.
2. **Do we want a v2 path that uses acme-dns for wildcard DNS-01?** Useful
   if a high-volume operator hits the per-week cert ceiling. The hook
   point in the `sni_acme` module is the challenge solver; supporting
   DNS-01 is a different challenger impl. Defer until someone asks.
3. **Operator config surface.** Today, `proxy.toml` lists a single
   domain. With on-demand, the proxy needs to know its zone root
   (`proxy.<domain>`) to validate incoming SNIs ("is `<x>.proxy.example`
   a sibling subdomain of mine?"). Add `wildcard_zone =
   "proxy.<domain>"` and validate inbound SNIs against it; refuse to
   provision certs for SNIs outside the zone. Prevents the proxy from
   being weaponised to mint certs for unrelated hosts that happen to
   resolve to its IP.
4. **Renewal observability.** Where do failed renewals surface?
   `samizdat doctor` is the natural home but it talks to the node only
   today; the proxy is a separate daemon. Either teach doctor to query
   the proxy's local admin port, or have the proxy write a status file
   and have doctor read it. Decide before implementation.
5. **Per-cert revocation.** If an operator wants to forget a series'
   cert (private key compromised at the series-owner level), an explicit
   admin route or just deleting the cache file? The latter is simpler
   and matches "operate by file" elsewhere in samizdat.

## Alternatives considered

- **acme-dns + DNS-01 wildcard cert.** Earlier draft of this RFC. Three
  operator-side steps instead of one. Better long-term for very-high-
  volume operators (no per-cert rate limits) but worse for the dead-simple
  bar this RFC was rescoped to hit. Kept as the v2 path for when someone
  needs it.
- **CDN in front (Cloudflare, Fastly).** Couples deployment to a CDN.
  Out of scope for a self-hostable project.
- **One cert with explicit Subject Alternative Names per series.** Cert
  has to be re-issued every time a series is added; brittle.
- **Skip the wildcard. Live with the proxy-side single-origin problem.**
  Status quo. T1/T9/T10/T15 stay reopened at the proxy; T16 remains.
  Costs zero, delivers nothing.
