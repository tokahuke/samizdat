//! Parses the `Host` header into one of the dispatch scopes the node
//! recognises. See `docs/upgrade-hazards.md` and `docs/browser-security.md`
//! for the motivation: each entity type gets its own browser origin via a
//! prefix-label subdomain of `localhost`, while all `/_*` admin routes stay
//! at the bare host.
//!
//! Trusted-host validation is the security-critical check: any Host header
//! other than bare loopback or `*.localhost` is rejected with 400, so an
//! attacker cannot spoof Host into something the routing logic would
//! mis-interpret.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use samizdat_common::identity::{check_servable_identity, Reason};
use samizdat_common::{Hash, Key};

/// Which origin the request is targeting. Produced by parsing the `Host`
/// header; consumed by content / admin handlers.
#[derive(Debug, Clone)]
pub enum HostScope {
    /// Bare `localhost`, `127.0.0.1`, or `[::1]` (with any port). The
    /// administrative origin; no series content lives here.
    BareLoopback,
    /// `series-<key>.localhost` -- series content (current edition).
    Series(Key),
    /// `object-<hash>.localhost` -- raw object bytes.
    Object(Hash),
    /// `collection-<hash>.localhost` -- item lookup inside a frozen
    /// content-addressed snapshot.
    Collection(Hash),
    /// `edition-<id>.localhost` -- item lookup inside a specific signed
    /// edition. The id is carried verbatim and resolved by the content
    /// handler.
    Edition(String),
    /// `<identity-handle>.localhost` -- identity content. The handle has
    /// been validated via `check_servable_identity` and is safe to use as
    /// the node-side identity-resolver input.
    Identity(String),
}

/// Why a `Host` header was rejected.
#[derive(Debug)]
pub enum HostScopeRejection {
    /// Request did not carry a `Host` header. HTTP/1.1 requires it; HTTP/2's
    /// `:authority` pseudo-header is normalised into `Host` by axum, so this
    /// only fires on a malformed client.
    MissingHost,
    /// Host header bytes were not valid UTF-8 or did not parse.
    Malformed(String),
    /// Host is something other than loopback / `*.localhost`. Trusted-host
    /// reject; the body names what was rejected so debug logs are useful.
    UntrustedHost(String),
    /// `series-<rest>` subdomain whose `<rest>` did not parse as a `Key`.
    BadSeriesKey(String),
    /// `object-<rest>` subdomain whose `<rest>` did not parse as a `Hash`.
    BadObjectHash(String),
    /// `collection-<rest>` subdomain whose `<rest>` did not parse as a `Hash`.
    BadCollectionHash(String),
    /// `edition-<rest>` subdomain with empty `<rest>`.
    BadEditionId(String),
    /// Subdomain treated as an identity handle and rejected by
    /// `check_servable_identity`. Carries the underlying `Reason` so the
    /// rendered 400 response tells the user what to fix.
    UnservableIdentity(Reason),
}

impl IntoResponse for HostScopeRejection {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            HostScopeRejection::MissingHost => {
                (StatusCode::BAD_REQUEST, "missing Host header".to_owned())
            }
            HostScopeRejection::Malformed(host) => (
                StatusCode::BAD_REQUEST,
                format!("malformed Host header: {host}"),
            ),
            HostScopeRejection::UntrustedHost(host) => (
                StatusCode::BAD_REQUEST,
                format!(
                    "untrusted Host header: {host}. samizdat-node only serves \
                     loopback and *.localhost hosts."
                ),
            ),
            HostScopeRejection::BadSeriesKey(label) => (
                StatusCode::BAD_REQUEST,
                format!("subdomain `{label}` has the `series-` prefix but the rest is not a valid key"),
            ),
            HostScopeRejection::BadObjectHash(label) => (
                StatusCode::BAD_REQUEST,
                format!("subdomain `{label}` has the `object-` prefix but the rest is not a valid hash"),
            ),
            HostScopeRejection::BadCollectionHash(label) => (
                StatusCode::BAD_REQUEST,
                format!("subdomain `{label}` has the `collection-` prefix but the rest is not a valid hash"),
            ),
            HostScopeRejection::BadEditionId(label) => (
                StatusCode::BAD_REQUEST,
                format!("subdomain `{label}` has the `edition-` prefix but is missing the id"),
            ),
            HostScopeRejection::UnservableIdentity(reason) => (
                StatusCode::BAD_REQUEST,
                format!(
                    "identity is not servable as a subdomain: {reason}. \
                     Use the typed form `series-<key>.localhost` instead, or pick a \
                     DNS-safe handle."
                ),
            ),
        };
        Response::builder()
            .status(status)
            .body(body.into())
            .expect("can build HostScope rejection")
    }
}

impl<S: Send + Sync> FromRequestParts<S> for HostScope {
    type Rejection = HostScopeRejection;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let raw = parts
            .headers
            .get("host")
            .ok_or(HostScopeRejection::MissingHost)?;
        let host_str = std::str::from_utf8(raw.as_bytes())
            .map_err(|_| HostScopeRejection::Malformed("non-utf8 bytes".to_owned()))?;
        classify(host_str)
    }
}

/// Parse + classify a raw `Host` header value (`host[:port]`, possibly with
/// IPv6 brackets). Exposed for unit tests; the extractor delegates here.
pub fn classify(raw: &str) -> Result<HostScope, HostScopeRejection> {
    let authority = http::uri::Authority::try_from(raw.trim())
        .map_err(|_| HostScopeRejection::Malformed(raw.to_owned()))?;
    let host = authority.host().to_ascii_lowercase();

    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return Ok(HostScope::BareLoopback);
    }

    let Some(subdomain) = host.strip_suffix(".localhost") else {
        return Err(HostScopeRejection::UntrustedHost(host));
    };
    if subdomain.is_empty() {
        return Err(HostScopeRejection::Malformed(raw.to_owned()));
    }
    // Multi-label subdomains (`a.b.localhost`) are out: `check_servable_identity`
    // rejects `.` in handles, and the prefix-label dispatch lives in one label.

    if let Some(rest) = subdomain.strip_prefix("series-") {
        return rest
            .parse::<Key>()
            .map(HostScope::Series)
            .map_err(|_| HostScopeRejection::BadSeriesKey(subdomain.to_owned()));
    }
    if let Some(rest) = subdomain.strip_prefix("object-") {
        return rest
            .parse::<Hash>()
            .map(HostScope::Object)
            .map_err(|_| HostScopeRejection::BadObjectHash(subdomain.to_owned()));
    }
    if let Some(rest) = subdomain.strip_prefix("collection-") {
        return rest
            .parse::<Hash>()
            .map(HostScope::Collection)
            .map_err(|_| HostScopeRejection::BadCollectionHash(subdomain.to_owned()));
    }
    if let Some(rest) = subdomain.strip_prefix("edition-") {
        if rest.is_empty() {
            return Err(HostScopeRejection::BadEditionId(subdomain.to_owned()));
        }
        return Ok(HostScope::Edition(rest.to_owned()));
    }

    check_servable_identity(subdomain)
        .map(|()| HostScope::Identity(subdomain.to_owned()))
        .map_err(HostScopeRejection::UnservableIdentity)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_label() -> String {
        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(&[3u8; 32]);
        Key::from(sk.verifying_key()).to_string()
    }

    #[test]
    fn classifies_bare_localhost() {
        assert!(matches!(classify("localhost").unwrap(), HostScope::BareLoopback));
        assert!(matches!(classify("localhost:4510").unwrap(), HostScope::BareLoopback));
        assert!(matches!(classify("127.0.0.1:4510").unwrap(), HostScope::BareLoopback));
        assert!(matches!(classify("[::1]:4510").unwrap(), HostScope::BareLoopback));
        assert!(matches!(classify("[::1]").unwrap(), HostScope::BareLoopback));
    }

    #[test]
    fn classifies_series_subdomain() {
        let label = key_label();
        let host = format!("series-{label}.localhost:4510");
        let scope = classify(&host).expect("valid series subdomain");
        assert!(matches!(scope, HostScope::Series(_)));
    }

    #[test]
    fn classifies_identity_subdomain() {
        let scope = classify("samizdat-blog.localhost:4510").expect("valid identity");
        assert!(matches!(scope, HostScope::Identity(h) if h == "samizdat-blog"));
    }

    #[test]
    fn rejects_untrusted_host() {
        let err = classify("evil.example:4510").unwrap_err();
        assert!(matches!(err, HostScopeRejection::UntrustedHost(_)));
    }

    #[test]
    fn rejects_unservable_identity_subdomain() {
        let err = classify("bank.example.localhost:4510").unwrap_err();
        assert!(
            matches!(err, HostScopeRejection::UnservableIdentity(_)),
            "expected UnservableIdentity, got {err:?}"
        );
    }

    #[test]
    fn rejects_bad_series_key() {
        let err = classify("series-not-a-key.localhost:4510").unwrap_err();
        assert!(matches!(err, HostScopeRejection::BadSeriesKey(_)));
    }

    #[test]
    fn upper_case_host_is_normalised_to_lower() {
        let scope = classify("LOCALHOST").expect("uppercase localhost");
        assert!(matches!(scope, HostScope::BareLoopback));
    }
}
