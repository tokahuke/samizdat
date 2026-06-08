# T3 audit: simple-POST cross-origin reachability

## Method

A browser cross-origin "simple POST" arrives at the node with no Authorization
header, no custom headers, and (if the attacker page declares
`Referrer-Policy: no-referrer`) no Referer header either. axum's `Json`
extractor does not enforce Content-Type, so a body labelled `text/plain` is
accepted as JSON as long as the bytes parse.

This pass walked every router under `node/src/http/` that defines a
state-mutating method (POST, PUT, PATCH, DELETE), recorded the auth gate, and
checked whether the gate fails closed against an unauthenticated cross-origin
simple POST.

Three gates appear in the codebase:

1. `authenticate_trusted_context` (auth.rs:618). Requires either a bearer
   admin token OR a Referer whose path is in `is_trusted_context` (only
   `/_register`). Both branches require a header that disqualifies the
   request from being a simple POST (Authorization) or that an attacker
   page cannot forge (Referer pointing at the admin host). Closes the
   surface.
2. `security_scope!(<scope>; <rights...>)` (auth.rs:534, expanding to
   `authenticate_security_scope`, auth.rs:505). Passes if EITHER the
   bearer-token branch succeeds at `<scope>` OR the entity from the Referer
   has any of `<rights>` granted. Importantly, `do_authenticate_security_scope`
   (auth.rs:572) ALWAYS appends `AccessRight::Public` to the entity's granted
   rights (auth.rs:599) before the membership check. Consequence: if
   `<rights>` includes `AccessRight::Public`, the middleware passes without
   either header. If `<rights>` is something more specific (e.g.
   `ManageObjects`), a request with no Referer collapses to
   `granted = [Public]`, fails the membership test, returns
   `InsufficientPrivilege`, and (combined with `MissingAuthorization` from
   the bearer branch) returns 401/403.
3. `SecurityScope` extractor (auth.rs:316). Used in the handler signature
   (kvstore.rs is the only caller). Calls `referer_from_parts` first; with
   no Referer, returns `MissingReferer` and the handler short-circuits with
   401.

Cross-origin attacker pages cannot forge a Referer pointing at
`localhost`/`*.localhost`: the browser sets it from the page's own origin,
and the `check_origin` predicate (auth.rs:182) rejects everything else as
`BadOrigin`. The only attacker-controllable Referer state from a cross-origin
page is "absent" (via `Referrer-Policy: no-referrer` or
`<meta name="referrer" content="no-referrer">`).

The audit therefore reduces to: does any mutating route survive an
"absent Referer, absent Authorization" request? That is exactly the set of
routes whose gate is `security_scope!(...; AccessRight::Public)` AND whose
handler does NOT additionally extract `SecurityScope` (which would fail with
`MissingReferer`).

## Route inventory

Mutating routes across the admin nest. Path column shows the full route
including the `nest("/_xxx", ...)` prefix from `mod.rs:188`.

| Route | Method | Gate | Body shape | Mutates | File:line |
|---|---|---|---|---|---|
| `/_kvstore/{*tail}` | PUT | `security_scope!(AccessRight::Public)` + handler `SecurityScope` extractor | `Json<PutRequest>` | Y | kvstore.rs:26, kvstore.rs:84-97 |
| `/_kvstore/{*tail}` | DELETE | `security_scope!(AccessRight::Public)` + handler `SecurityScope` extractor | none | Y | kvstore.rs:31, kvstore.rs:100-109 |
| `/_kvstore/` | DELETE | `security_scope!(AccessRight::Public)` + handler `SecurityScope` extractor | none | Y | kvstore.rs:35, kvstore.rs:112-121 |
| `/_objects/` | POST | `security_scope!(AccessRight::ManageObjects)` | raw bytes | Y | objects.rs:71-97 |
| `/_objects/{hash}` | DELETE | `security_scope!(AccessRight::ManageObjects)` | none | Y | objects.rs:98-104 |
| `/_objects/{hash}/reissue` | POST | `security_scope!(AccessRight::ManageObjects)` | none | Y | objects.rs:105-125 |
| `/_objects/{hash}/bookmark` | POST | `security_scope!(AccessRight::ManageBookmarks)` | none | Y | objects.rs:130-144 |
| `/_objects/{hash}/bookmark` | DELETE | `security_scope!(AccessRight::ManageBookmarks)` | none | Y | objects.rs:164-177 |
| `/_collections/` | POST | `security_scope!(AccessRight::ManageCollections)` | `Json<PostCollectionRequest>` | Y | collections.rs:51-80 |
| `/_series-owners/` | POST | `security_scope!(AccessRight::ManageSeries)` | `Json<PostSeriesOwnerRequest>` | Y | series_owners.rs:60-89 |
| `/_series-owners/{nickname}` | DELETE | `security_scope!(AccessRight::ManageSeries)` | none | Y | series_owners.rs:102-117 |
| `/_series-owners/{nickname}/editions` | POST | `security_scope!(AccessRight::ManageSeries)` | `Json<PostEditionRequest>` | Y | series_owners.rs:123-203 |
| `/_subscriptions/` | POST | `security_scope!(AccessRight::ManageSubscriptions)` | `Json<PostSubscriptionRequest>` | Y | subscriptions.rs:27-44 |
| `/_subscriptions/{key}` | DELETE | `security_scope!(AccessRight::ManageSubscriptions)` | none | Y | subscriptions.rs:64-78 |
| `/_hubs/` | POST | `security_scope!(AccessRight::ManageHubs)` | `Json<PostHubRequest>` | Y | hubs.rs:37-54 |
| `/_hubs/{hub}` | DELETE | `security_scope!(AccessRight::ManageHubs)` | none | Y | hubs.rs:69-85 |
| `/_ethereum-provider/` | PUT | `security_scope!()` (admin, no entity right) | `Json<PutEthereumProviderRequest>` | Y | ethereum_provider.rs:30-43 |
| `/_auth/{*tail}` | PATCH | `authenticate_trusted_context` | `Json<Request>` | Y | auth.rs:123-152 |
| `/_auth/{*tail}` | DELETE | `security_scope!()` (admin, no entity right) | none | Y | auth.rs:157-176 |
| `/_vacuum/` | POST | `authenticate_trusted_context` | none | Y | mod.rs:309-312, 323 |
| `/_vacuum/flush-all` | POST | `authenticate_trusted_context` | none | Y | mod.rs:313-322, 323 |

`editions.rs`, `series.rs`, `connections.rs`, `peers.rs`, `content.rs`,
`redirects.rs`, `host_scope.rs`, `identities.rs`, `resolvers.rs` define no
mutating routes (GET only or no routes at all), so they are not in the table.

## Findings

No flagged rows.

Reasoning by gate class:

* `authenticate_trusted_context` (4 mutating routes: `/_vacuum/`,
  `/_vacuum/flush-all`, `/_auth/{*tail}` PATCH). Both branches require a
  header an attacker page cannot supply: bearer auth in the header bag
  forces a CORS preflight, and the trusted Referer path requires
  `http(s)://(localhost|*.localhost)/_register` which only a same-origin
  page on the admin host can produce. Safe.

* `security_scope!()` and `security_scope!(admin;)` with empty rights
  array (2 mutating routes: `/_ethereum-provider/` PUT, `/_auth/{*tail}`
  DELETE). The entity-rights path needs ANY of an empty array, so the
  `granted_rights.iter().any(...)` test (auth.rs:602-606) is vacuously
  false even after Public is appended. Bearer is the only path, and a
  bearer header is not a simple POST. Safe.

* `security_scope!(AccessRight::Manage*)` rights (12 mutating routes
  across objects, collections, series-owners, subscriptions, hubs).
  With no Referer, the appended `Public` does not satisfy
  `[ManageX]`, so `do_authenticate_security_scope` returns
  `InsufficientPrivilege`. With a cross-origin Referer, `check_origin`
  returns `BadOrigin`. With a forged Referer pointing at an attacker
  domain the browser will not let the page lie. The bearer branch is
  also missing. `merge_rejections` (auth.rs:438) yields a 401 / 403 and
  the handler never runs. Safe.

* `security_scope!(AccessRight::Public)` (3 mutating routes, all in
  `/_kvstore`). The middleware DOES pass for a no-Referer no-Authorization
  request (Public is unconditionally granted at auth.rs:599). The saving
  grace is that each kvstore handler signature also extracts
  `SecurityScope(entity): SecurityScope`. That extractor calls
  `referer_from_parts` and returns `Err(SecurityScopeRejection::MissingReferer)`
  when the Referer is absent (auth.rs:319-322), producing a 401 before any
  DB write. So a cross-origin simple POST is rejected at the handler-extractor
  step rather than at the middleware step. The route is closed but only by
  the handler. See "Open questions" below.

## Clean rows

The 21 rows in the inventory above were each checked against the gate
classification. No row reaches its mutating logic from an Authorization-less
no-Referer cross-origin POST. Read-only GET routes were not enumerated; per
the task statement they are out of scope.

The `kvstore` routes are listed as clean because the handler-side
`SecurityScope` extractor fails closed, but they are the only routes in the
entire admin nest where the layered gate (`security_scope!(...; Public)`) is
strictly weaker than the handler-side check. A future refactor that drops
the explicit `SecurityScope(...)` from the kvstore handler signature (say,
because the entity is reconstructed elsewhere) would silently open the
surface. Worth a comment in `kvstore.rs` saying "this extractor is
load-bearing for cross-origin defense".

## Open questions

1. The kvstore handlers' defense-in-depth: should the public-facing gate be
   tightened from `security_scope!(AccessRight::Public)` to something that
   refuses requests without a Referer at the middleware layer, so the
   handler extractor stops being load-bearing? The cleanest expression
   would be a dedicated `security_scope!(referrer_required; AccessRight::Public)`
   form whose `do_authenticate_security_scope` returns
   `MissingReferer` when `entity_from_request` returns `Ok(None)` and
   `<rights>` includes `Public`. This is a code-shape question, not a
   bug; flagging it for the deferred backlog if the maintainers agree the
   defense-in-depth is worth the macro complication.

2. `Referrer-Policy: same-origin` is set as a default response header
   (mod.rs:228), but that only constrains what THIS server's responses
   tell browsers about THEIR Referer behavior for subsequent navigations
   from pages served by us. It does not constrain a cross-origin attacker
   page's own Referrer-Policy. The audit assumed attacker pages can and
   will set `no-referrer`. Confirm that assumption with the threat-model
   author if there is any doubt.
