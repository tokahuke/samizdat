# Proxy DNS-01 provider design

## Goal and scope

The proxy obtains a wildcard TLS certificate for `*.proxy.<domain>` so it
can terminate TLS for every series subdomain (`<base32-key>.proxy.<domain>`)
and every identity subdomain (`<identity>.proxy.<domain>`) without
demanding a per-name HTTP-01 round trip on first hit. Wildcard issuance
forces DNS-01, which means the proxy must write and remove a TXT record
at the operator's authoritative DNS provider during each renewal. The
proxy ships built-in support for DigitalOcean, Cloudflare, and Route53;
operators on other providers implement the trait in-tree and link it in.
Per Pedro's constraint, no `aws-*`, `cloudflare-*`, or `digitalocean-*`
SDK enters the dependency tree: all three providers are spoken to over
plain HTTPS via the existing `reqwest` client, including a small in-tree
SigV4 signer for Route53.

## Trait

A single `DnsProvider` trait lives in `proxy/src/dns/mod.rs`. It is
async (the rest of the codebase is tokio) and uses `async fn` in trait
directly rather than `impl Future` return-position sugar; the trait is
only ever dyn-dispatched behind an `Arc<dyn DnsProvider + Send + Sync>`
owned by the wildcard cert manager, so the desugaring cost is paid once
per renewal, not per request.

The method set:

* `async fn set_txt(&self, zone: &str, record_name: &str, value: &str)
  -> Result<TxtHandle, DnsError>`. The `zone` is the apex of the zone
  the operator configured (for example `hubfederation.com` or
  just `hubfederation.com`, whichever the operator's DNS provider
  treats as a hosted zone). The `record_name` is the fully qualified
  name to create, typically `_acme-challenge.proxy.<domain>`. The
  `value` is the ACME-computed token; the implementation is responsible
  for any quoting the provider's API requires. There is no TTL hint
  parameter: each implementation sets the lowest TTL the provider
  permits (60 seconds across all three), because the record only lives
  for the duration of a single ACME poll cycle.
* `async fn remove_txt(&self, zone: &str, handle: TxtHandle)
  -> Result<(), DnsError>`. Takes back the `TxtHandle` returned by
  `set_txt` and deletes the record. Best-effort: see the orphan-record
  stance below.
* `async fn check_zone(&self, zone: &str) -> Result<(), DnsError>`.
  Called once at proxy startup. The trait ships a default
  implementation that calls `set_txt` followed by `remove_txt` with a
  sentinel name (`_samizdat-preflight.<zone>`) and a random value.
  Implementations override only if they have a cheaper smoke test;
  none of the three built-ins do, so they inherit the default.
  Refusing to boot when this fails is preferable to discovering the
  misconfiguration 60 days later when the cert is about to expire.

The `TxtHandle` is an opaque in-process wrapper around a `String` that
carries the provider-specific record identifier (DO record id,
Cloudflare record id; for Route53 the handle carries the value
verbatim because Route53's DELETE takes name and value, not an id).
The handle is not persisted to disk: see the orphan-record stance
below.

The `set_txt` call must NOT block on DNS propagation. ACME servers do
their own DNS polling after we tell them to validate; if the proxy also
blocks on a global DNS view, we double the wall-clock for no benefit
and risk timing out the ACME client. The cert manager calls `set_txt`,
waits for the provider's own propagation-by-API confirmation (DO and
Cloudflare both return synchronously once the change is in their
authoritative servers; Route53 returns a `ChangeId` that the manager
polls via `GetChange` to `INSYNC` before yielding control), and then
hands off to `instant-acme` to do the cross-internet poll.

`remove_txt` is best-effort, pure and simple. On the happy path the
cert manager calls `set_txt`, runs the ACME validation, then calls
`remove_txt`. If `remove_txt` returns an error, the manager logs at
`warn` level and moves on. There is no on-disk journal, no startup
sweep, no User-Agent marker.

Orphans accumulate slowly in the operator's DNS provider when
`remove_txt` fails repeatedly or when the proxy crashes between
`set_txt` and `remove_txt`. This is harmless for ACME: each renewal
asks for a fresh challenge token, the new TXT record carries that new
value, and the ACME server matches against the value it issued. Stale
records with old values are dead bytes; they do not affect cert
issuance. An operator who cares about DNS-record hygiene can periodically
purge `_acme-challenge.<wildcard-root>` TXT records older than a day
from their DNS console; samizdat does not do this automatically because
the infrastructure cost of doing it correctly outweighs the cost of
ignoring the orphan drift.

The error type is deliberately minimal: a two-variant enum
`DnsError { Transport(reqwest::Error), Provider(String) }`. The cert
manager does not vary its behaviour by error category today; all it
does with an error is log and retry with backoff. Cataloguing
`Unauthorized` / `NotFound` / `RateLimited` separately is premature
without evidence the renewal scheduler needs to branch on them. Grow
the enum when an implementation surfaces a variant that the manager
genuinely has to handle differently (the obvious future case is
`RateLimited` once Let's Encrypt hits the 50-cert-per-week ceiling and
the manager needs to back off harder); until then, both variants
collapse to the same retry-with-backoff loop.

## Implementations

Each provider lives in its own module under `proxy/src/dns/`:
`proxy/src/dns/digitalocean.rs`, `proxy/src/dns/cloudflare.rs`,
`proxy/src/dns/route53.rs`, `proxy/src/dns/script.rs`. All four share
`proxy/src/dns/mod.rs` for the trait, the `TxtHandle`, and the
`DnsError` enum. The Route53 signer is `proxy/src/dns/aws_sigv4.rs`.

### DigitalOcean

Base URL: `https://api.digitalocean.com/v2`. Auth: a single Bearer
token in the `Authorization` header. The operator passes the same
`do_token` that Terraform already holds, or (preferred for hardening) a
narrowed PAT scoped to `domain:read, domain:create, domain:delete` on
the specific zone, as already noted in `docs/operations.md`.

Zone discovery: DigitalOcean addresses zones by their apex name
directly in the URL path, so there is no lookup step. The
`zone` config key is the apex (`hubfederation.com`). The challenge
record name passed to `set_txt` is the fully qualified
`_acme-challenge.hubfederation.com`, and the implementation
strips the trailing `.<zone>` before posting, since the DO API expects
the relative name in the `name` JSON field.

The minimum API surface, per challenge:

* `POST /v2/domains/<zone>/records` with body
  `{"type": "TXT", "name": "_acme-challenge.proxy", "data": "<token>",
  "ttl": 60}`. Response is JSON `{"domain_record": {"id": <i64>, ...}}`.
  The `id` is the `TxtHandle` payload.
* `DELETE /v2/domains/<zone>/records/<id>`. Empty body, 204 on success,
  404 if already gone (treated as Ok).

### Cloudflare

Base URL: `https://api.cloudflare.com/client/v4`. Auth: a Bearer API
token in the `Authorization` header. Pedro's stated constraint: only
the modern scoped API tokens are supported, not the legacy global API
key + `X-Auth-Email` pair. The token requires `Zone:Read` and
`DNS:Edit` on the target zone.

Zone discovery: Cloudflare addresses zones by an opaque `zone_id`
hex string. At startup `check_zone` does `GET /zones?name=<zone>`
and caches the returned id. The cached id is invalidated and re-fetched
if any subsequent call returns a 404 on the zone path.

The minimum API surface:

* `POST /zones/<zone_id>/dns_records` with body
  `{"type":"TXT","name":"<fqdn>","content":"<token>","ttl":60}`.
  Response is JSON `{"result":{"id":"<record_id>",...},"success":true}`.
  The `record_id` is the `TxtHandle` payload.
* `DELETE /zones/<zone_id>/dns_records/<record_id>`. 200 on success,
  404 treated as Ok.

### Route53 (incl. self-contained SigV4 signer notes)

Base URL: `https://route53.amazonaws.com`. Auth: AWS SigV4 with the
operator's access-key id and secret access key. Both keys are
config-resident; no instance-profile detection, no `~/.aws/credentials`
loading. This is intentional: the proxy is a daemon with a known
deployment shape, and threading the AWS credential-resolver fan-out
into the proxy is exactly the SDK-shaped complexity Pedro's constraint
rules out.

Zone discovery: Route53 addresses zones by an opaque hosted-zone-id
(`/hostedzone/<id>`). At startup `check_zone` calls
`GET /2013-04-01/hostedzonesbyname?dnsname=<zone>&maxitems=1` and
caches the returned id. The response is XML and is parsed with a
hand-written 30-line scan rather than a full XML parser dependency
(the field set is small, fixed, and not user-controlled).

The minimum API surface, per challenge:

* `POST /2013-04-01/hostedzone/<zone_id>/rrset/` with an XML body
  shaped as a `ChangeResourceRecordSetsRequest` containing a single
  `Change` of `Action=UPSERT`, `ResourceRecordSet` with
  `Name=_acme-challenge.proxy.<zone>.`, `Type=TXT`, `TTL=60`, and a
  single `ResourceRecord` whose `Value` is the token wrapped in
  literal double quotes (Route53 stores TXT values quoted). Response
  XML carries a `ChangeInfo` with an `Id` like `/change/C123ABC`.
* `GET /2013-04-01/change/<change-id>` polled until `Status=INSYNC`,
  to satisfy the "wait for provider-side propagation" contract before
  we hand control to `instant-acme`. Bounded by a 90-second deadline;
  on timeout we yield anyway and let the ACME poll do the rest.
* `POST /2013-04-01/hostedzone/<zone_id>/rrset/` with
  `Action=DELETE`, full record body byte-identical to the UPSERT, for
  cleanup. Route53 requires the DELETE payload to exactly match the
  existing record, which is why the `TxtHandle` for Route53 carries
  the literal value rather than a record id.

The SigV4 signer lives in `proxy/src/dns/aws_sigv4.rs` and is scoped
narrowly to what these calls need:

* Service `route53`, region `us-east-1` (Route53 is a global service
  but its SigV4 region is fixed at `us-east-1`).
* Canonical request: HTTP method, URI path (already percent-encoded,
  no double-encoding pass on top), canonical query string (sorted
  by key, percent-encoded, equals sign retained on empty values),
  canonical headers (lowercase names, trimmed values, sorted, each
  terminated by `\n`), signed headers list (lowercase names sorted,
  semicolon-joined), and the hex SHA-256 of the request body
  (`UNSIGNED-PAYLOAD` is not used; payloads are small).
* String to sign: `AWS4-HMAC-SHA256\n<amz-date>\n<credential-scope>\n
  <hex-sha256-of-canonical-request>`.
* Signing-key derivation: `kDate = HMAC(kSecret, date)`,
  `kRegion = HMAC(kDate, "us-east-1")`,
  `kService = HMAC(kRegion, "route53")`,
  `kSigning = HMAC(kService, "aws4_request")`.
* Required headers added by the signer: `Host`, `X-Amz-Date`,
  `Authorization`. No session-token path: STS credentials are out of
  scope (see the credential-resolver paragraph above).

HMAC-SHA256 and SHA-256 come from `ring`, which the workspace already
pulls in transitively via `rustls-acme`. No new crate is added. The
signer module exposes a single function that takes a mutable
`reqwest::Request` and the credentials, computes the signature, and
sets the three headers. It has unit tests using the
publicly-documented test vectors from the AWS SigV4 specification.

Route53-specific quirks the signer accounts for:

* The version-prefixed path `/2013-04-01/...` is part of the canonical
  URI and is signed verbatim.
* The XML body, not JSON, is hashed for the payload digest. The body
  is built by string templating against a fixed XML skeleton; no
  reflection-based serialiser involved.
* The trailing `.` on the FQDN inside the XML is required by Route53
  and is part of the signed body.

## Config shape

A new `[dns]` block in `proxy.toml`. The shape is a discriminated
subblock: `[dns.digitalocean]`, `[dns.cloudflare]`, `[dns.route53]`,
`[dns.script]`. Subblocks rather than a tagged-union flat key set
because (a) the credential shapes do not overlap, and (b) a tagged
union would let `serde` silently accept a Cloudflare-shaped token
under `provider = "digitalocean"`.

Exactly one subblock is present when wildcard TLS is enabled. A
top-level `[dns]` block with no subblock disables the wildcard path
(the proxy still does HTTP-01 against the bare name).

The `zone` and `wildcard_root` keys are common to all subblocks and
live at the `[dns]` level:

* `zone` is the apex of the hosted zone the credentials can write
  inside (`hubfederation.com`).
* `wildcard_root` is the name the wildcard cert covers
  (`hubfederation.com`); the cert SANs are
  `hubfederation.com` and `*.hubfederation.com`. Defaults
  to the existing `domain` value if omitted.

**Credentials live in env vars, not in `proxy.toml`.** TOML files get
read out of band, backed up, accidentally committed; env vars are
process-scoped and rotate by editing one file and restarting the
service. The TOML carries only the non-secret topology. Each provider
ships with a default env-var name matching the operator's existing
tooling; the TOML can override the name if the operator runs multiple
proxies with different sets:

* DigitalOcean: `DIGITALOCEAN_TOKEN` (matches the DO CLI and
  `terraform-provider-digitalocean`).
* Cloudflare: `CLOUDFLARE_API_TOKEN` (matches `flarectl` and
  Cloudflare's CLI docs).
* Route53: `AWS_ACCESS_KEY_ID` plus `AWS_SECRET_ACCESS_KEY` plus
  optional `AWS_SESSION_TOKEN` (standard AWS env-var set; Route53
  operators already have these).

Deployment plumbing: the `samizdat-proxy` systemd unit ships with
`EnvironmentFile=/etc/samizdat/proxy.env` (mode `0640`, owner
`samizdat:samizdat`). The operator populates that one file; systemd
injects the vars at start. Rotation is "edit the file, `systemctl
restart samizdat-proxy`".

### Example: DigitalOcean

```toml
[dns]
zone = "hubfederation.com"
wildcard_root = "hubfederation.com"

[dns.digitalocean]
# token_env = "MY_OTHER_NAME"   # optional override; defaults to DIGITALOCEAN_TOKEN
```

`/etc/samizdat/proxy.env`:

```
DIGITALOCEAN_TOKEN=dop_v1_...
```

### Example: Cloudflare

```toml
[dns]
zone = "hubfederation.com"
wildcard_root = "hubfederation.com"

[dns.cloudflare]
# token_env = "MY_OTHER_NAME"   # optional override; defaults to CLOUDFLARE_API_TOKEN
```

`/etc/samizdat/proxy.env`:

```
CLOUDFLARE_API_TOKEN=<scoped API token>
```

### Example: Route53

```toml
[dns]
zone = "hubfederation.com"
wildcard_root = "hubfederation.com"

[dns.route53]
region = "us-east-1"
# access_key_id_env     = "MY_OTHER_ACCESS_KEY"
# secret_access_key_env = "MY_OTHER_SECRET_KEY"
# session_token_env     = "MY_OTHER_SESSION_TOKEN"   # all optional overrides
```

`/etc/samizdat/proxy.env`:

```
AWS_ACCESS_KEY_ID=AKIA...
AWS_SECRET_ACCESS_KEY=...
AWS_SESSION_TOKEN=...                # only if using STS temp creds
```

### Example: Script (escape hatch for unsupported providers)

The script provider exists for operators on DNS providers we do not
ship native support for (Hetzner, Linode, Gandi, BIND-via-nsupdate,
their own homegrown panel). The operator points the proxy at one or
two scripts and we shell out per challenge. The interface matches
certbot's `--manual-auth-hook` / `--manual-cleanup-hook` so existing
certbot DNS hooks work without modification.

```toml
[dns]
zone = "example.com"
wildcard_root = "proxy.example.com"

[dns.script]
set    = "/etc/samizdat/dns-hooks/set"
delete = "/etc/samizdat/dns-hooks/delete"
# Or one binary that branches on the action:
# command = "/etc/samizdat/dns-hook"   # invoked with SAMIZDAT_DNS_ACTION=set|delete
# timeout_seconds = 30                  # optional; default 30
```

The script is invoked with these env vars set:

* `SAMIZDAT_DNS_ZONE` -- the configured zone apex.
* `SAMIZDAT_DNS_NAME` -- the FQDN to operate on (e.g.
  `_acme-challenge.proxy.example.com`).
* `SAMIZDAT_DNS_VALUE` -- the TXT value (the ACME token).
* `SAMIZDAT_DNS_ACTION` -- `set` or `delete`. Set when the single-
  command form is used.

Any env vars present in the proxy's own environment (notably whatever
the operator's script needs for its own provider auth) are inherited
verbatim. The script's stdout/stderr is captured and logged at the
proxy's tracing level; non-zero exit is a `DnsError::Provider(stderr
trimmed)`. Timeout enforced by the proxy via `tokio::process` with
the configured `timeout_seconds`.

`TxtHandle` for the script provider carries the value verbatim; the
delete invocation receives the same `SAMIZDAT_DNS_VALUE` so the
script can scope the deletion if its DNS provider distinguishes
records by value.

The existing top-level keys in `Cli` (`data`, `node`, `https`,
`port`, `http_port`, `owner`, `acme_directory`) are unchanged. The
`Cli` struct gains an `Option<DnsConfig>` field with
`#[serde(default)]`, so legacy configs without a `[dns]` block keep
working with HTTP-01 only.

## Integration

The HTTP-01 path that `proxy/src/acme.rs::serve` runs today against the
bare `domain` name stays. The new wildcard path is additive and sits
alongside it.

The integration shape:

* A `WildcardCertManager` struct in `proxy/src/wildcard.rs` owns
  `Arc<dyn DnsProvider>`, the wildcard cert cache directory, and an
  `instant_acme::Account` it persists to disk at
  `<acme-cache>/wildcard/account.json`. `instant-acme` is the only
  new dependency in this layer; the trait plumbing is by-hand but the
  ACME state machine is not.
* The cert and its key live at `<acme-cache>/wildcard/cert.pem` and
  `<acme-cache>/wildcard/key.pem`. The DirCache shape that
  `rustls-acme` uses for the bare-name HTTP-01 cert is preserved as-is
  under `<acme-cache>/`; the wildcard cache is a sibling directory,
  not a replacement.
* Renewal scheduler: a single tokio task spawned next to the existing
  rustls-acme task in `serve`. It runs a loop on a 12-hour ticker
  that checks the on-disk cert's NotAfter and triggers a renewal
  when fewer than 30 days remain. Same cadence rustls-acme picks
  internally for the bare name; matching it keeps the logging shape
  uniform.
* SNI dispatch: the rustls-acme `axum_acceptor` already builds a
  `ResolvesServerCert` for the bare name. The wildcard manager
  implements its own `ResolvesServerCert`, and the proxy composes the
  two with a small wrapper resolver: incoming SNI exactly equal to
  the bare `domain` is served by the rustls-acme resolver; everything
  matching `*.<wildcard_root>` is served by the wildcard resolver;
  anything else gets the wildcard cert too, so a misrouted request
  still completes the TLS handshake (the HTTP layer will then 404).
  The composed resolver is the single value passed into
  `axum_server::bind(addr).acceptor(...)`.
* `instant-acme` exposes `Order::authorizations` and per-challenge
  `KeyAuthorization::dns_value`; the manager loops over each
  authorization, calls `provider.set_txt(zone, name, dns_value)`,
  calls `Order::set_challenge_ready`, polls `Order::poll` until
  `OrderStatus::Ready`, finalises with a CSR for the wildcard SANs,
  downloads the certificate chain, atomically renames it into place,
  then walks the handles in reverse calling `provider.remove_txt`
  (best-effort; failures logged at `warn` and ignored).

The boot order in `proxy/src/main.rs` becomes: parse config, call
`provider.check_zone(zone)` if a `[dns]` block is present, refuse to
boot on failure, then proceed into `acme::serve` with the composed
resolver installed. This means a misconfigured `[dns]` block manifests
as a hard startup failure with a clear log line, not as a silent
fall-through to HTTP-01 (which would then fail the wildcard issuance
60 days later).

## Failure modes and recovery

* **Proxy crashes between create and delete.** The TXT record stays
  orphaned in the operator's DNS. This is harmless for ACME (each
  new renewal asks for a fresh challenge token; stale TXT records
  with old values do not affect validation) and the proxy makes no
  attempt to clean it up automatically. An operator who cares about
  DNS-record hygiene can periodically purge old `_acme-challenge`
  records from their DNS console.
* **DNS provider rate-limits or 5xx.** Each provider call is wrapped
  in a bounded exponential backoff: initial 2 seconds, factor 2, max
  5 attempts, cap at 60 seconds, jitter 25%. A persistent failure
  surfaces as `DnsError::Provider(...)` to the cert manager, which
  logs at `error` level and re-enters the 12-hour renewal loop. The
  proxy keeps serving the existing cert no matter how far past
  validity it is; once expiry is within 7 days the renewal task
  escalates the log to a per-cycle `error` line ("cert expires in
  N days, renewal failing") that an operator running `journalctl -u
  samizdat-proxy | grep -i 'cert expires'` will see. Decision:
  silent-degradation matches the existing HTTP-01 path; the wildcard
  cert does not change that policy.
* **Cert renews fine but provider rejects the delete.** Log at
  `warn` level with the `(zone, handle)` pair, leave the pending
  journal entry in place, and continue serving the freshly issued
  cert. The next startup replay or the next `check_zone` sweep
  removes the orphan. The renewal is considered successful: a stale
  TXT record is harmless to subsequent ACME validations because each
  renewal upserts its own.
* **Configured `zone` does not match a zone the credentials can
  write.** The `check_zone` preflight at startup attempts the
  sentinel record cycle; on any error it returns a
  `DnsError::Provider(...)` (or `DnsError::Transport` if the call
  could not be made at all). The proxy logs `dns provider preflight
  failed for zone '<zone>': <error>` at `error` and exits with a
  non-zero status, mirroring the existing `validate_node_is_up` shape
  in `proxy/src/http.rs`. systemd restart-on-failure will not fix
  this; the operator must fix the config.

## Out of scope

* HTTP-01 against the bare `proxy.<domain>` name is unchanged. The
  rustls-acme code path in `proxy/src/acme.rs::serve` keeps running
  and keeps owning that one cert. The `[dns]` block is opt-in; an
  operator who only needs single-host TLS does not configure it and
  gets exactly today's behaviour.
* Providers other than DigitalOcean, Cloudflare, Route53, and the
  built-in script escape hatch can be added by the operator without
  re-implementing the trait in Rust: the `[dns.script]` provider
  shells out to a configured set of commands using the
  certbot-compatible env-var hook interface (see config example).
  Operators who want a native Rust implementation for performance or
  to avoid the shell can drop a file under `proxy/src/dns/<provider>.rs`,
  impl `DnsProvider`, add a match arm to `DnsConfig`, add a subblock
  to the config schema; no dynamic plugin loading. The trait
  contract is this document.
* Cloudflare legacy auth (global API key + `X-Auth-Email`) is not
  supported. Only Bearer-shaped scoped API tokens. Operators on
  legacy auth rotate to a scoped token; the Cloudflare dashboard has
  done this for years and the legacy path costs us a second auth
  shape in the implementation for no gain.

## Open questions

None outstanding. Decisions baked into the design:

- Wildcard scope is one label deep: `*.<wildcard_root>` plus the bare
  `<wildcard_root>`. Nested `*.<key>.<wildcard_root>` is out.
- Route53 credentials are long-lived `AWS_ACCESS_KEY_ID` plus
  `AWS_SECRET_ACCESS_KEY`. STS is out.
- DO token scope is not introspected at boot; the proxy uses whatever
  token the operator hands it.
- No on-disk state for DNS-01: no journal, no startup sweep. `remove_txt`
  is best-effort; orphan TXT records are tolerated and the operator
  cleans them up out of band if they care.
