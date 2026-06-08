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
//! `blockchain/SamizdatIdentity.sol` (the contract source) was tightened to
//! reject the same shapes in `registerWithTtl`; the live deployment is
//! unchanged until redeployed. See `blockchain/REDEPLOY.md`. Until that
//! happens this module is the only line of defense against new garbage
//! registrations, and it remains the only defense against pre-redeploy
//! garbage forever.

use crate::host_label::KEY_HOST_LABEL_LEN;

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
    /// Length and alphabet exactly match a 52-char base32 series-key label;
    /// would collide with key-vs-identity dispatch in
    /// `node/src/http/host_scope.rs`.
    #[error(
        "identity matches the 52-character base32 series-key shape and would \
         collide with key-based subdomain dispatch"
    )]
    KeyShape,
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
/// shadow or be confused with infrastructure / reserved-tld names. The list
/// mirrors what the amended `SamizdatIdentity.sol` rejects on-chain.
const RESERVED_LABELS: &[&str] = &[
    "localhost",
    "local",
    "arpa",
    "test",
    "example",
    "invalid",
    "localhost4",
    "localhost6",
    "samizdat",
];

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

    if RESERVED_LABELS.iter().any(|r| *r == s) {
        return Err(Reason::Reserved);
    }

    if bytes.iter().all(|b| b.is_ascii_digit()) {
        return Err(Reason::AllNumeric);
    }

    // Collision with the 52-char base32 key shape. The base32 lowercase
    // alphabet [a-z2-7] is a strict subset of the [a-z0-9-] we accept above,
    // so the check is just length + digit-restriction.
    if s.len() == KEY_HOST_LABEL_LEN && bytes.iter().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
    {
        return Err(Reason::KeyShape);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_accepts_realistic_identities() {
        for ok in &["samizdat-blog", "alice", "blog-2026", "abc", "a-b-c-d", "x", "get-samizdat"]
        {
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
        assert_eq!(check_servable_identity("-alice"), Err(Reason::LeadingHyphen));
        assert_eq!(check_servable_identity("alice-"), Err(Reason::TrailingHyphen));
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
    fn rejects_key_shape_collision() {
        // 52 chars, all in base32 lowercase alphabet [a-z2-7].
        let collision = "a".repeat(KEY_HOST_LABEL_LEN);
        assert_eq!(check_servable_identity(&collision), Err(Reason::KeyShape));
        // 52 chars containing a digit outside [2-7] passes (8 is not base32).
        let mut not_collision = "a".repeat(KEY_HOST_LABEL_LEN - 1);
        not_collision.push('8');
        assert!(check_servable_identity(&not_collision).is_ok());
    }
}
