# RFC: wildcard cert for the public proxy

## Status

Draft. Author: leave the byline empty. Targeted release: post-0.3.x; this
unlocks the proxy-side per-series-origin isolation that closes T1/T9/T10/T15
reopens at `proxy.hubfederation.com` and folds T16 (shared proxy
`localStorage` key).

## Motivation

`docs/proxy-app-divergence.md` catalogues four threats that reopen at the
proxy origin because every series is served under one host
(`proxy.hubfederation.com`). The structural fix is to serve each series under
`<base32-key>.proxy.<domain>` and each identity under
`<handle>.proxy.<domain>`, mirroring what `node/src/http/host_scope.rs`
already does on the local node. That requires a wildcard TLS cert for
`*.proxy.<domain>` plus a wildcard DNS record pointing at the proxy host.

ACME wildcard certs require the DNS-01 challenge (HTTP-01 cannot prove
control of a wildcard). DNS-01 requires writing a `TXT` record at
`_acme-challenge.proxy.<domain>` whenever Let's Encrypt asks for renewal.
Writing that record means talking to the operator's DNS provider, and every
provider has its own API. Coupling the proxy code to one provider is the
"too coupled to do" cost.

## The agnostic-DNS-API question

The closest vendor-agnostic answer is **acme-dns**: a tiny purpose-built
authoritative DNS server (open source, BSD, maintained) whose only job is to
serve `TXT` records for the ACME DNS-01 challenge. The proxy talks to
acme-dns's stable two-endpoint HTTP API instead of a per-provider SDK; the
operator's real DNS provider stays out of the picture except for ONE
permanent CNAME record.

Setup pattern:

1. Operator runs an acme-dns instance (or uses a trusted public one).
2. Operator sets a one-time CNAME in their real DNS:
   `_acme-challenge.proxy.<domain>` -> `<random>.auth.<acme-dns-host>`.
3. The proxy stores acme-dns credentials (returned by acme-dns at
   first-time `/register`) in its config and renews the wildcard cert
   on schedule by `POST`ing to acme-dns's `/update`.

This works regardless of whether the operator's main zone is at Cloudflare,
Route53, Hetzner DNS, NS1, or a BIND instance in their basement. The only
constraint is "can set one CNAME in their DNS provider".

Alternatives considered and rejected as the primary path:

- **RFC 2136 + TSIG** is the IETF-standard dynamic update protocol; BIND,
  PowerDNS, and Knot all support it. Managed DNS providers like Cloudflare
  and Route53 do not expose it. Only useful if the operator runs their own
  authoritative nameservers. Worth supporting as a secondary path.
- **lego / certbot DNS plugins** ship per-provider adapters. The
  abstraction is at the tool level; the codebase still has to choose
  providers to support. Pragmatic for end users but not "agnostic at the
  proxy".
- **Provider-specific Rust SDKs** is what most projects end up doing.
  N HTTP clients, N config tables, N test matrices. The cost is exactly the
  coupling the RFC is trying to avoid.

## Goal

After this RFC ships:

- The proxy serves a TLS cert valid for `proxy.<domain>` and
  `*.proxy.<domain>`.
- A wildcard DNS A/AAAA record for `*.proxy.<domain>` points at the proxy
  host (operator does this once in their real DNS).
- Path-form external URLs (`https://proxy.<domain>/_series/<base64-key>/<path>`
  and `/~<identity>/<path>`) continue to work but redirect to the host-form
  (`https://<base32-key>.proxy.<domain>/<path>` and
  `https://<identity>.proxy.<domain>/<path>`).
- The proxy is otherwise unchanged in behaviour. The host-form URLs travel
  the same `translate_to_node_url` path internally that the path-form did
  (see `proxy/src/http.rs::translate_to_node_url`).

## Design

### Current code

- `proxy/Cargo.toml:39-43` depends on `rustls-acme = "0.12.1"`.
- `proxy/src/acme.rs` wires `AcmeConfig` and runs the renewal stream.
- `proxy/src/http.rs::translate_to_node_url` rewrites incoming path-form
  URLs to host-form upstream against the local node.
- TLS-ALPN-01 and HTTP-01 are the only challenges `rustls-acme` 0.12 ships
  out of the box; DNS-01 is not supported by the crate today.

### Library choice

Three viable Rust crates support ACME DNS-01:

- **`instant-acme`** (https://crates.io/crates/instant-acme). Async,
  pluggable challenge solver. Recommended.
- **`acme-lib`**. Sync; less idiomatic with the rest of the proxy.
- **Shelling out to `lego` or `certbot`**. Works but introduces a
  binary dependency on the host.

Recommendation: pull in `instant-acme` for the wildcard path, keep
`rustls-acme` for the existing non-wildcard cert. Two ACME flows side by
side is messier than swapping outright, BUT swapping requires re-doing
the working renewal pipeline; staged migration is safer. See "Open
questions" below.

### Config additions

`proxy/src/cli.rs` (and the corresponding TOML) gets a new optional
section:

```toml
[wildcard]
acme_dns_url      = "https://auth.example/"
acme_dns_user     = "<from acme-dns /register>"
acme_dns_pass     = "<from acme-dns /register>"
acme_dns_subdomain = "<from acme-dns /register>"
wildcard_domain   = "proxy.<domain>"
```

`wildcard_domain` is the wildcard root; the cert SANs become
`<wildcard_domain>` and `*.<wildcard_domain>`.

When the `[wildcard]` block is absent, the proxy behaves as today: single
cert for `proxy.<domain>`. This makes the feature opt-in and keeps the
testbed running untouched until the operator explicitly turns it on.

### acme-dns client

A new `proxy/src/acme_dns.rs` module exposing the minimum surface:

```rust
pub struct AcmeDnsClient {
    base_url: Url,
    user: String,
    pass: String,
    subdomain: String,
}

impl AcmeDnsClient {
    pub async fn set_txt(&self, value: &str) -> Result<()>;
}
```

That is the entire HTTP API needed for the DNS-01 challenge. One `POST
{base}/update` with the `X-Api-User` / `X-Api-Key` headers and a JSON body
containing `{ subdomain, txt }`.

### Wiring into the ACME flow

`instant-acme`'s `Account::order` returns a list of `Authorization`s, each
with a list of `Challenge`s. The proxy picks the `dns-01` variant, asks
`AcmeDnsClient::set_txt(&token)`, polls for propagation (acme-dns serves
it immediately because acme-dns IS the authoritative nameserver for the
delegated subdomain), then signals Let's Encrypt to validate. Standard
flow; the only proxy-specific code is the acme-dns call.

### URL rewriting changes

`proxy/src/http.rs::translate_to_node_url` already handles the host-form
internally for the upstream node call. The external surface change:

- Bare `proxy.<domain>/` keeps the welcome / docs path (no change).
- `proxy.<domain>/_series/<base64>/<rest>` -> 301 to
  `https://<base32>.proxy.<domain>/<rest>` (and serves directly when
  arrived at the host-form).
- `proxy.<domain>/~<identity>/<rest>` -> 301 to
  `https://<identity>.proxy.<domain>/<rest>`.
- The 301 redirect ensures old shared links keep working but the user's
  browser lands on the per-series origin, closing T1/T9/T10/T15 reopens.

`translate_to_node_url` becomes a no-op rewrite for the host-form incoming
requests: the proxy receives `<base32>.proxy.<domain>/foo`, sets the
upstream URL `<base32>.localhost:4510/foo` directly.

### Operational rollout

1. Operator stands up an acme-dns instance (own machine or a trusted
   public one). Costs nothing if self-hosted; a t2.micro is overkill.
2. Operator sets the `_acme-challenge.proxy.<domain>` CNAME.
3. Operator sets the `*.proxy.<domain>` wildcard A/AAAA pointing at the
   proxy host.
4. Operator drops the `[wildcard]` config block into the proxy's
   `proxy.toml`.
5. samizdat-proxy is restarted; instant-acme provisions the wildcard
   cert; renewal happens on schedule.

No data migration, no client breaking change. External users keep typing
`https://proxy.hubfederation.com/_series/<key>/...` and get a 301 to the
new host-form on first hit; the 301 lasts forever, browsers cache it,
shared links propagate naturally.

## Security considerations

- **acme-dns trust**. If the acme-dns instance is compromised, an
  attacker can forge ACME challenges for `*.proxy.<domain>` and obtain a
  fraudulent cert for any series subdomain. The operator should self-host
  acme-dns where possible; the public free instance is acceptable for
  personal projects but not for the testbed. Document this.
- **Wildcard scope**. The wildcard cert covers `<anything>.proxy.<domain>`.
  An identity-name typo cannot escape that namespace. A wildcard does NOT
  cover multiple labels (`<a>.<b>.proxy.<domain>` is uncovered) which
  matches our flat-subdomain expectation.
- **DNS-01 vs HTTP-01 separation**. Keeping the existing `rustls-acme`
  HTTP-01 flow for the non-wildcard cert means the proxy needs both port
  443 (for serving) and port 80 (for HTTP-01) bound. No change from
  today.
- **Renewal failure mode**. If acme-dns is unreachable during a renewal
  attempt, instant-acme retries with backoff. If the cert is within 30
  days of expiry and renewals keep failing, alarm; do not just serve a
  stale cert silently. Add a `samizdat doctor` field for "days until
  proxy cert expiry".

## Open questions

1. **Swap rustls-acme out entirely, or stage in instant-acme alongside?**
   Two ACME flows in one binary is messy; one flow is cleaner but the
   migration touches the working renewal path. Recommend staged.
2. **Self-host acme-dns or use the public instance for the testbed?** The
   testbed is at `proxy.hubfederation.com`; self-hosting acme-dns on
   that same machine (or a sibling) is cheap and avoids depending on a
   third-party uptime. Recommend self-host.
3. **Should the 301 redirect path-form -> host-form be permanent, or
   serve both shapes indefinitely so links stay simple?** Permanent
   redirect is the cleaner long-term posture (one canonical form);
   indefinite dual-serving costs nothing functionally but blurs the URL
   shape. Decision: permanent 301, keep the path-form parser only to
   issue the redirect.
4. **Do we want a fallback to RFC 2136 / TSIG for operators who DO run
   their own authoritative DNS?** The cost is one more code path. Keep
   for v2 unless an operator asks.
5. **Renewal observability.** Where do failed renewals surface?
   `samizdat doctor` is the natural home but `proxy` is a different
   daemon than `node`; doctor talks to the node only today. Either teach
   doctor to query the proxy's local API too, or have the proxy write a
   status file and have doctor read it. Decide before implementation.

## Alternatives considered

- **Skip wildcard cert; live with the proxy-side single-origin
  problem.** Status quo. T1/T9/T10/T15 stay reopened at the proxy.
  T16 (shared `__samizdat_proxy_page_count`) remains. Costs zero,
  delivers nothing.
- **Use a per-provider Rust ACME plugin matrix.** Tedious to maintain,
  couples the codebase to a fixed set of providers, fragile when
  providers change their APIs.
- **Front the proxy with a CDN that handles wildcard ACME for you
  (Cloudflare, Fastly).** Couples deployment to a CDN. Out of scope for
  a self-hostable project; mention but reject.
