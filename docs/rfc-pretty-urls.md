# RFC: pretty samizdat URLs

## Status

draft. Author: . Targeted release: post-0.3.x.

## Motivation

Samizdat-node currently serves content at per-series subdomains under the
reserved `localhost` TLD, on a non-default HTTP port. A real URL today
reads:

```
http://r0km0hpteta6fhosmy7qxakxydtwhkzi0eybt1watdm.localhost:4510/index.html
http://samizdat-blog.localhost:4510/posts/2026-06-01
```

The 52-char base32 key is unavoidable -- it is the Ed25519 public key for
that series and is what makes the URL self-authenticating. The rest of
the URL is overhead we can shave off. `localhost` is doing one job
(routing the request to loopback without hitting DNS) and is loudly
visible while doing it. `:4510` is doing another (avoiding the privileged
port range) and is just as loud. Neither helps a human reading,
sharing, or remembering a samizdat link. The status quo also leaks the
implementation detail that samizdat is a local process to anyone who
glances at an address bar.

This RFC proposes routing the same content at `<sub>.samizdat` (no port).
That is, two cosmetic changes layered on top of the existing per-series
subdomain dispatcher (`node/src/http/host_scope.rs`, summarised in
`docs/upgrade-hazards.md` item 6): a different host-suffix for the
loopback rewrite, and an opt-in privileged-port bind so the port can be
elided. The series-isolation and identity-resolution work the
subdomain dispatch was built for is unchanged; what changes is only the
DNS shape on the user's machine and the daemon's listen port.

## Goal

- A samizdat node serves at `<base32-key>.samizdat` (no port) and
  `<identity>.samizdat` (no port) from the user's browser.
- The bare loopback admin host stays accessible. Either via `samizdat`
  (no subdomain, treated as the admin scope by the existing host
  dispatcher) or via a distinct host such as `admin.samizdat`. See open
  questions.
- Opt-in: a user runs `samizdat-up dns install` to enable. The default
  install does NOT touch system DNS; the loopback `:4510` path keeps
  working unchanged for users who never opt in.
- Removable: `samizdat-up dns uninstall` reverses every per-OS change
  this RFC introduces. The inverse of every install side effect is
  documented per platform.

## Design overview

Two independent pieces, gated to the same opt-in:

1. A local DNS responder (or a static system-resolver entry, where the
   OS supports one) that maps `*.samizdat` and the bare `samizdat`
   label to `127.0.0.1` / `::1`. Nothing else resolves through it.
2. An opt-in change to how the node binds its HTTP listener: instead of
   port 4510 it binds port 80, either via a Linux capability, a
   privileged-service arrangement, or (on Windows) the SCM-elevated
   default.

The two are decoupled in the implementation -- the DNS rewrite gets a
user from "`localhost:4510`" to "`samizdat:4510`", and only the port
change drops the `:4510`. A user who wants the cleaner host but not the
privileged bind can take just the first. The CLI surface (`samizdat-up
dns install`, `samizdat-up dns install --bind-80`) reflects that.

The current node port flag at `node/src/cli.rs:26-28` is unchanged by
this RFC; the bind-80 path is a separate config knob (or a samizdat-up
unit-file/plist override) so the flag's default stays portable.

## Per-OS install plumbing

### macOS

- The hook: write `/etc/resolver/samizdat` containing
  `nameserver 127.0.0.1` and `port <dns-port>`. macOS's `mDNSResponder`
  consults `/etc/resolver/<tld>` to redirect queries for that TLD at a
  named resolver without touching any global resolver configuration.
- The daemon: a `samizdat-dns` binary, registered through launchd the
  same way the existing daemons are. The plist install path mirrors
  `samizdat-up/src/install/macos.rs:308-321` (`write_plist`) and the
  reverse-DNS label scheme at
  `samizdat-up/src/install/macos.rs:323-328` (`plist_path`,
  `/Library/LaunchDaemons/com.samizdat.dns.plist`). Service registration
  uses the same `launchctl bootstrap system <plist>` + `launchctl enable
  system/<label>` pattern as
  `samizdat-up/src/install/macos.rs:63-83`.
- Uninstall: `launchctl bootout system/com.samizdat.dns`, then remove
  the plist and the `/etc/resolver/samizdat` file. The uninstall path
  mirrors the existing flow at
  `samizdat-up/src/install/macos.rs:90-124`.

### Linux

The Linux path branches on what the host's resolver stack looks like.
Detection runs once at `samizdat-up dns install` time and picks one of
three modes.

- **dnsmasq present.** Drop `/etc/dnsmasq.d/samizdat.conf` with a
  single line:
  ```
  address=/.samizdat/127.0.0.1
  ```
  Then `systemctl restart dnsmasq`. No samizdat-dns daemon needed; the
  mapping is static. This is the cheapest mode and the one to prefer
  when available. The conf write lands using the same convention as
  `ensure_config` in `samizdat-up/src/install/linux.rs:298-338`
  (atomic write, preserve user edits if file already exists).
- **systemd-resolved present.** Drop
  `/etc/systemd/resolved.conf.d/samizdat.conf`:
  ```
  [Resolve]
  DNS=127.0.0.1:<dns-port>
  Domains=~samizdat
  ```
  Then `systemctl restart systemd-resolved`. `Domains=~samizdat`
  routes queries for the `samizdat` domain at the configured DNS
  server only; everything else still goes upstream. Open question:
  whether systemd-resolved's static-map config (no daemon required)
  can express `*.samizdat -> 127.0.0.1` directly, or whether the
  samizdat-dns daemon is needed even here. The straightforward
  reading of the resolved man page is that `~samizdat` routing
  requires a DNS server on the other side; the daemon ships either
  way for parity with macOS.
- **Neither.** Two options:
  - Install the samizdat-dns daemon and append a NetworkManager dispatch
    script that patches `/etc/resolv.conf` on link-up; or
  - Fail with an actionable error pointing the operator at the
    documented dnsmasq / systemd-resolved setup. Failing closed is
    safer; mutating `/etc/resolv.conf` directly tends to be undone by
    every subsequent DHCP lease renewal.

The samizdat-dns daemon, when installed, gets a systemd unit named
`samizdat-dns.service` written through the same path as the existing
daemons (`write_unit_file` at
`samizdat-up/src/install/linux.rs:340-347`, rendered via
`daemons::render_systemd_unit`). Service registration uses the same
`systemctl enable --now` pattern as
`samizdat-up/src/install/linux.rs:73-76`.

Uninstall removes the dnsmasq / resolved conf drop-in, removes the
unit file, runs `systemctl daemon-reload`, and on the neither-case
removes the NetworkManager dispatch script. This mirrors
`samizdat-up/src/install/linux.rs:228-269`.

### Windows

- The hook: `Add-DnsClientNrptRule -Namespace ".samizdat" -NameServers
  127.0.0.1`. NRPT (Name Resolution Policy Table) lets the DNS client
  route queries for a namespace at a specific resolver. samizdat-up
  shells out to `powershell.exe -Command` from
  `samizdat-up dns install`.
- The daemon: register the `samizdat-dns` binary with the SCM through
  the same wrapper used by the existing daemons. The `sc.exe create
  binPath= "...samizdat-up.exe daemon dns"` call follows the pattern
  at `samizdat-up/src/install/windows.rs:400-427` (`sc_create`). The
  SCM-side wrapper / supervisor is unchanged
  (`samizdat-up/src/install/windows.rs:481-635`); samizdat-dns becomes
  one more known component handled by the existing service_main.
- Uninstall: `Remove-DnsClientNrptRule` for the namespace, then `sc.exe
  stop` and `sc.exe delete` of the samizdat-dns service. Mirrors
  `samizdat-up/src/install/windows.rs:129-162`.

## The samizdat-dns daemon

A small process (target: 100-200 lines of Rust + dep on `hickory-server`
or similar) that:

- Binds `127.0.0.1:<port>` (and `::1:<port>`) for UDP and TCP, the two
  transports DNS uses. Port is configurable; default is in the
  unprivileged range so the daemon does not need
  `CAP_NET_BIND_SERVICE`.
- Answers `A` and `AAAA` queries for any name that is exactly
  `samizdat.` or that ends in `.samizdat.`, with `127.0.0.1` and `::1`
  respectively. TTL: short (60 seconds), so a user who uninstalls
  doesn't have stale resolver cache entries lingering.
- Returns `NXDOMAIN` for anything else. This is the load-bearing
  property for the leak argument in the security section below.

Where this daemon lives in samizdat-up:

- It becomes a fourth entry in `samizdat-up/src/daemons.rs::ALL` (today
  the slice is `&[&NODE, &HUB, &PROXY]` at `daemons.rs:49`), with
  `bin = "samizdat-dns"`, a default config that names the DNS port,
  and the same `render_systemd_unit` / `render_launchd_plist`
  treatment the other three get for free (`daemons.rs:88` / `:147`).
  The `KNOWN_BINARIES` slice at `daemons.rs:55` grows by one.
- It opts in to the same `Component` plumbing in
  `samizdat-up/src/cli.rs` so `samizdat-up install dns`, `samizdat-up
  list`, and `samizdat-up uninstall dns` all flow naturally. The `dns
  install` subcommand is a thin wrapper that calls `install_component`
  with `Component::Dns` plus the per-OS hook writer (the resolver
  file / dnsmasq conf / NRPT rule).

Tradeoff: on Linux dnsmasq and systemd-resolved boxes, the daemon could
be avoided entirely -- a static map in the existing resolver covers
the use case. The cost of that asymmetry is two code paths to maintain
(daemon mode + static-map mode) and two uninstall sequences. The
simpler design ships the daemon everywhere, accepts a few extra MB of
RSS on Linux hosts that already have a resolver, and writes the
dnsmasq / resolved conf to point at it (or, alternately, treats it as
a no-op and relies purely on the static map). The RFC defers the final
choice to the implementer; both shapes pass the same end-to-end test.

## Port binding (separate decision, gated to the same opt-in)

The default port remains 4510 so a no-flags install does not need root
to bind. The port-80 path is opt-in via `samizdat-up dns install
--bind-80` (or `samizdat-up node bind-80`, depending on how the CLI
naming shakes out).

- **Linux.** `setcap cap_net_bind_service=+ep /usr/local/bin/samizdat-node`
  during install. The daemon keeps running as the unprivileged user
  named in the systemd unit (today `User=root` by default per
  `daemons::render_systemd_unit` at `daemons.rs:147`, configurable to
  `--as-user`); the capability lets it bind a port below 1024 without
  any other privilege. Re-running `setcap` after every binary replace
  is required (the capability is on the inode, lost on overwrite); the
  install_daemon_binary path at
  `samizdat-up/src/install/linux.rs:271-284` would gain a single
  `setcap` shell-out after the atomic rename when port-80 mode is
  active.
- **macOS.** macOS has no `CAP_NET_BIND_SERVICE` equivalent. Options:
  - Run the launchd daemon as root (the LaunchDaemons default; the
    plist already supports this -- the `UserName` block in
    `render_launchd_plist` at `daemons.rs:88-132` is only emitted when
    `as_user` is `Some`). Simple, but means the node runs as root.
  - Use an authbind-style helper. Heavier, custom code; not worth it.
  - Configure launchd's `Sockets` key to have launchd itself bind port
    80 (privileged) and hand the listening FD to the daemon
    (unprivileged). This is the cleanest macOS-native option and is
    worth investigating before settling on "run as root".
- **Windows.** SCM services run elevated by default; binding port 80
  needs no extra ceremony.

The `--bind-80` flag flips the unit-file / plist / SCM args to use
port 80 (and on Linux, runs the setcap). It does NOT mutate
`/etc/samizdat/node.toml` -- the daemon config keeps the port as a
plain value; the override lives in the service definition so that
toggling bind-80 doesn't conflict with the
"configs are never overwritten" guarantee documented in
`docs/upgrade-hazards.md` item 5.

## Security and operational considerations

- **ICANN delegation risk.** ICANN could in principle delegate
  `.samizdat` as a real gTLD some day. Extremely unlikely for a name
  this specific, and the local resolver override pre-empts the real
  resolver anyway; the only harm in that hypothetical would be that
  real `.samizdat` names worldwide stop resolving on this host. The
  smart contract at `blockchain/SamizdatIdentity.sol` already reserves
  the literal `samizdat` against on-chain squatting, which is a
  separate concern (identity registration) but rhymes with this one.
- **Browser TLD heuristics.** Chrome's omnibox sometimes treats
  unknown TLDs as search queries -- typing
  `r0km0hpteta6fhosmy7qxakxy....samizdat` may end up at Google
  instead of the local resolver. Firefox is less aggressive.
  Implementer should smoke-test the major browsers once the install
  path works and document any required user gestures (typing
  `http://` explicitly, hitting Ctrl+Enter, etc.).
- **DNS leak.** The samizdat-dns daemon answers ONLY `*.samizdat`
  queries and the bare `samizdat.` label; everything else is
  NXDOMAIN. The OS resolver knows from the per-platform routing
  config (`/etc/resolver/samizdat`, `Domains=~samizdat`, NRPT
  namespace) to only consult it for those names. So adding the
  daemon does NOT change what upstream DNS sees for any non-samizdat
  query, and a NXDOMAIN from samizdat-dns for an unexpected `.samizdat`
  name does not leak the query upstream either (the OS does not
  retry a Domain-scoped query against another resolver). State this
  explicitly in the install confirmation banner.
- **Privileged listener.** `--bind-80` widens the attack surface
  modestly: a bug in the HTTP handler is now reachable on the
  well-known port from any process on the same loopback. The node
  already binds loopback only; this RFC does not change that.
- **Operationally: clean uninstall.** Every install side effect has
  a documented inverse. The uninstall test is "after `samizdat-up
  dns uninstall`, `dig samizdat-blog.samizdat` returns NXDOMAIN from
  the system resolver, no samizdat-dns process is listening, and
  port 80 is no longer bound."

## Rollout

- **Phase 1: macOS.** `/etc/resolver/samizdat` plus the daemon. The
  smallest implementation surface (one resolver-file write, one
  plist) and the lowest blast radius (a single OS, with a
  well-documented per-TLD resolver mechanism).
- **Phase 2: Linux dnsmasq path.** Static map, no daemon. Cheapest of
  the Linux variants and the one most likely to be in place on a
  developer's box.
- **Phase 3: Linux systemd-resolved path.** Covers stock Ubuntu /
  Fedora / Arch. Decides the daemon-vs-static question for this mode.
- **Phase 4: Windows NRPT.** PowerShell shell-out + an SCM service.
- **Phase 5: port 80 opt-in.** Across all four platforms. Independent
  enough that it could ship earlier; ordering it last keeps each
  earlier phase one-piece-at-a-time.

Each phase is independently shippable; a partial rollout where macOS
has pretty URLs and Linux still uses `:4510` is fine.

## Alternatives considered

- **Stay on `<sub>.localhost:4510` forever.** The status quo. Ugly
  but operationally free. Acceptable as the default for users who do
  not opt in; not acceptable as the only option.
- **Use `.local` (mDNS).** `.local` is reserved by RFC 6762 for
  multicast DNS service discovery on the LAN. Anything binding it
  for unicast resolution risks collisions with Bonjour, Avahi, and
  every other mDNS responder on the network. Not the right fit.
- **Use a real owned TLD with wildcard DNS pointing to 127.0.0.1
  (e.g. `*.s.hubfederation.com -> 127.0.0.1`).** Works locally
  without any per-OS plumbing; the user does no install. Requires
  public DNS records under our domain and a wildcard ACME cert if
  HTTPS is wanted. Mentioned but rejected as a Phase 0: it fits more
  naturally with the public-proxy wildcard-cert track and ties
  per-host content URLs to an external DNS dependency we would
  rather not have. Worth revisiting if `*.samizdat` turns out
  unworkable in practice.
- **Per-browser extension that rewrites the URL bar.** Lossy
  (extensions are per-browser, per-profile, easy to disable) and
  intrusive (asks for tab-content permissions). Worse user
  experience than DNS for the same outcome.

## Open questions

- Bare-admin-host shape. Is the admin scope `samizdat` (no subdomain,
  matching how `localhost:4510` is already the admin host today) or
  a separate `admin.samizdat`? The former is a smaller change in
  the host-scope dispatcher; the latter is more legible.
- Daemon-or-static-map on Linux. Is the samizdat-dns daemon mandatory
  on every platform (simple, uniform, two extra MB of RSS), or
  should the Linux dnsmasq / systemd-resolved paths be static maps
  with no daemon at all (cheaper, but two install modes)?
- macOS port-80 strategy. authbind-equivalent, run-as-root, or
  launchd `Sockets`-key FD handoff? The Sockets approach is the most
  Apple-native; needs a quick spike to confirm the daemon code can
  pick up an inherited listener cleanly.
- DNS port. `5354` is a common convention for local stub resolvers;
  is there a reason to prefer something else (collision with another
  product, etc.)? Should it be configurable in
  `/etc/samizdat/dns.toml`?
- Subscription identity URLs. Identities resolved via the on-chain
  registry (`samizdat-blog`, etc.) already work as
  `<identity>.localhost:4510`; this RFC carries them to
  `<identity>.samizdat` mechanically. Does anything in the identity
  layer assume the `.localhost` suffix (e.g. for cookie-scope or
  CORS origin checks)? Quick audit needed before Phase 1 lands.

## File this lives in

docs/rfc-pretty-urls.md
