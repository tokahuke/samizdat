# Roadmap

Where the project goes from here, in honest priority. Each step
unlocks the next; out of order they don't compose.

## 1. Pinner app

The structural fix for samizdat's biggest gap: the publisher's laptop
is the SPOF for the propagation window after every commit. Without
pinners, a publisher who isn't online when subscribers ask is invisible.

A pinner is a thin sidecar to a `samizdat-node`, the same shape as
`samizdat-proxy` -- doesn't own data, doesn't speak the federation
protocol, doesn't store anything itself. Its job is the control plane
the node lacks today: "who paid me to keep what alive, and for how
long." When payment lands it adds a `FullInventory` subscription via
the node's admin API; on expiry it drops the subscription. The node
does everything else.

V1 is no-Polygon (any HTTP `POST /api/pin` is fine for the demo),
single-operator (one pinner per operator), no proof-of-pinning yet.
Polygon, multi-operator discovery, and cryptographic pinning proofs
are V2+ work.

What this unlocks:
- Publishers can publish from a laptop and walk away.
- The federation has more than one publicly-reachable endpoint for any
  given content -- the takedown surface diffuses across operators.

## 2. Browser-only reader

Only meaningful AFTER (1). With one publicly-reachable proxy (today),
a WASM-in-browser reader has nowhere to fetch from except the same
proxy that already serves the page -- net new value is roughly zero.
With many pinners running in many jurisdictions, a browser-side
samizdat client can pick from several sources, verify content
addressing locally, and treat any individual pinner as untrusted.
*That* is when the install step dies and samizdat becomes a web
thing instead of a daemon thing.

Sketch: WASM crate exposes a JS-callable client; talks WebTransport
to one or more pinners; speaks the same bincode-over-tarpc wire as
nodes, just transport-shimmed. Verification (hash, edition signature)
happens client-side. Mobile becomes free because mobile browsers are
browsers.

## 3. Killer demo

When (1) and (2) are stable: pick a piece of content someone wants
offline, publish to several pinners across jurisdictions, let the
takedown attempts fail publicly. The samizdat moment the name
promises. Premature without (1)+(2) -- a failed demo costs more than
no demo.

## Out of scope (deliberately)

- A WASM reader that goes through the existing proxy. That's not a
  reader, that's the proxy with extra steps.
- Browsers as peers (WebRTC mesh between tabs). Browsers are
  transient; not a substrate to build on.
- Per-`/64` cap accounting, IPv6 hardening, ChannelId HMAC binding,
  any of the audit theater in `deferred.md`'s antibody catalog.
  Those don't gate anything on this roadmap.
