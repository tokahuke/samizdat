//! Identity-handle validation, stricter than what the smart contract enforces.
//!
//! The on-chain contract (`blockchain/SamizdatIdentity.sol::registerWithTtl`)
//! accepts any non-empty identity that does not start with `_`. That is far
//! more permissive than what the node can safely serve under
//! `<identity>.localhost:<port>`: a DNS label must be 1..=63 ASCII bytes from
//! `[a-z0-9-]`, with no leading or trailing hyphen. Identities that fail the
//! check here can still be LOOKED UP via `samizdat identity get` and resolved
//! to a series key by the node, but they cannot themselves be the host part
//! of a content URL.
//!
//! Identities also cannot start with a typed-subdomain marker prefix
//! (`object-`, `series-`, `collection-`, `edition-`); those collide with the
//! prefix-label dispatch used to address entities by hash or key.
//!
//! `blockchain/SamizdatIdentity.sol` (the contract source) was tightened to
//! reject the same shapes in `registerWithTtl`; the live deployment is
//! unchanged until redeployed. See `blockchain/REDEPLOY.md`. Until that
//! happens this module is the only line of defense against new garbage
//! registrations, and it remains the only defense against pre-redeploy
//! garbage forever.

/// Why a candidate identity handle is unservable as a subdomain.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Reason {
    #[error("identity is empty")]
    Empty,
    #[error("identity is {0} bytes; DNS labels are limited to 63 bytes")]
    TooLong(usize),
    /// Carries the offending byte so the user knows what to remove.
    #[error("{}", bad_byte_message(*.0))]
    BadByte(u8),
    #[error("identity starts with '-'")]
    LeadingHyphen,
    #[error("identity ends with '-'")]
    TrailingHyphen,
    #[error("identity is a reserved DNS label")]
    Reserved,
    #[error("identity is all digits; reserved against numeric host ambiguity")]
    AllNumeric,
    /// Carries the offending type-marker word so the error message tells the
    /// user which prefix triggered the rejection.
    #[error(
        "identity starts with the reserved type prefix '{0}-'; that namespace is \
         used to address objects, series, collections, and editions by hash"
    )]
    TypePrefix(&'static str),
}

fn bad_byte_message(b: u8) -> String {
    if b.is_ascii_graphic() {
        format!(
            "identity contains '{}' (0x{:02x}); only a-z, 0-9 and '-' are allowed",
            b as char, b
        )
    } else {
        format!(
            "identity contains a non-printable byte 0x{:02x}; only a-z, 0-9 and '-' are allowed",
            b
        )
    }
}

/// Labels that must never be registerable as identities because they would
/// shadow or be confused with infrastructure / reserved-tld names, or with
/// the typed-subdomain marker words used by the dispatch layer.
const RESERVED_LABELS: &[&str] = &[
    "localhost",
    "local",
    "arpa",
    "test",
    "example",
    "invalid",
    "localhost4",
    "localhost6",
    "object",
    "series",
    "collection",
    "edition",
];

/// Type-marker prefixes used by the dispatch layer. An identity matching
/// `<word>-<anything>` collides with the prefix-label hash/key dispatch and
/// is rejected here.
const RESERVED_TYPE_PREFIXES: &[&str] = &["object", "series", "collection", "edition"];

/// Returns `Ok` iff `s` is a "servable identity" for subdomain hosting.
/// Stricter than the smart contract; see the module docstring for the
/// motivation.
pub fn check_servable_identity(s: &str) -> Result<(), Reason> {
    if s.is_empty() {
        return Err(Reason::Empty);
    }
    if s.len() > 63 {
        return Err(Reason::TooLong(s.len()));
    }

    // Alphabet pass. ASCII only.
    for b in s.bytes() {
        let ok = matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-');
        if !ok {
            return Err(Reason::BadByte(b));
        }
    }

    let bytes = s.as_bytes();
    if bytes[0] == b'-' {
        return Err(Reason::LeadingHyphen);
    }
    if *bytes.last().expect("non-empty") == b'-' {
        return Err(Reason::TrailingHyphen);
    }

    if RESERVED_LABELS.contains(&s) {
        return Err(Reason::Reserved);
    }

    if bytes.iter().all(|b| b.is_ascii_digit()) {
        return Err(Reason::AllNumeric);
    }

    for prefix in RESERVED_TYPE_PREFIXES {
        // Match `<prefix>-<at least one more byte>`. The trailing-hyphen guard
        // above already rejects `<prefix>-` with no body.
        if s.len() > prefix.len() + 1
            && s.as_bytes().starts_with(prefix.as_bytes())
            && s.as_bytes()[prefix.len()] == b'-'
        {
            return Err(Reason::TypePrefix(prefix));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_accepts_realistic_identities() {
        for ok in &[
            "samizdat-blog",
            "alice",
            "blog-2026",
            "abc",
            "a-b-c-d",
            "x",
            "get-samizdat",
        ] {
            assert!(check_servable_identity(ok).is_ok(), "expected ok: {ok}");
        }
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(check_servable_identity(""), Err(Reason::Empty));
    }

    #[test]
    fn rejects_too_long() {
        let s = "a".repeat(64);
        assert_eq!(check_servable_identity(&s), Err(Reason::TooLong(64)));
    }

    #[test]
    fn rejects_dot() {
        match check_servable_identity("bank.example") {
            Err(Reason::BadByte(b'.')) => {}
            other => panic!("expected BadByte('.'); got {other:?}"),
        }
    }

    #[test]
    fn rejects_underscore() {
        match check_servable_identity("foo_bar") {
            Err(Reason::BadByte(b'_')) => {}
            other => panic!("expected BadByte('_'); got {other:?}"),
        }
    }

    #[test]
    fn rejects_uppercase() {
        match check_servable_identity("Alice") {
            Err(Reason::BadByte(b'A')) => {}
            other => panic!("expected BadByte('A'); got {other:?}"),
        }
    }

    #[test]
    fn rejects_unicode() {
        match check_servable_identity("café") {
            Err(Reason::BadByte(_)) => {}
            other => panic!("expected BadByte for non-ASCII; got {other:?}"),
        }
    }

    #[test]
    fn rejects_leading_and_trailing_hyphen() {
        assert_eq!(
            check_servable_identity("-alice"),
            Err(Reason::LeadingHyphen)
        );
        assert_eq!(
            check_servable_identity("alice-"),
            Err(Reason::TrailingHyphen)
        );
    }

    #[test]
    fn rejects_reserved_labels() {
        for r in RESERVED_LABELS {
            assert_eq!(check_servable_identity(r), Err(Reason::Reserved), "{r}");
        }
    }

    #[test]
    fn accepts_punycode_prefix() {
        assert!(check_servable_identity("xn--caf-dma").is_ok());
    }

    #[test]
    fn rejects_all_numeric() {
        assert_eq!(check_servable_identity("12345"), Err(Reason::AllNumeric));
        // Mixed letters and digits passes.
        assert!(check_servable_identity("abc123").is_ok());
    }

    #[test]
    fn accepts_52_char_base32_now_that_keyshape_is_gone() {
        // The prefix-label dispatch keeps series keys under `series-<key>`,
        // so a bare 52-char base32 string can register as an identity.
        let s = "a".repeat(52);
        assert!(check_servable_identity(&s).is_ok());
    }

    #[test]
    fn rejects_type_prefix() {
        for prefix in RESERVED_TYPE_PREFIXES {
            let candidate = format!("{prefix}-something");
            assert_eq!(
                check_servable_identity(&candidate),
                Err(Reason::TypePrefix(prefix)),
                "expected TypePrefix rejection for {candidate}"
            );
        }
    }

    #[test]
    fn accepts_type_prefix_without_hyphen() {
        // `objectalike` does not collide with `object-<hash>` dispatch.
        assert!(check_servable_identity("objectalike").is_ok());
    }
}
