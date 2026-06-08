//! Encoding a [`Key`] as a DNS label, for per-series subdomain hosting.
//!
//! samizdat-node serves each series at `<host-label>.localhost:<port>` so the
//! browser treats each series as its own origin. The wire-level identifier
//! stays the 32-byte Ed25519 public key (encoded as URL-safe base64); the
//! "host label" defined here is only the dispatch-time representation that
//! fits the DNS-label alphabet ([a-z0-9-], 1..=63 chars). We use RFC 4648
//! base32 lowercase without padding so the label can be reused verbatim as a
//! DNS subdomain.
//!
//! 32-byte key -> 52 base32 chars, comfortably under the 63-char DNS label
//! limit. Identity handles also live in this namespace; see
//! [`is_base32_key_label`] for how the two are disambiguated at parse time.

use data_encoding::{Encoding, Specification};

use crate::Key;

/// Length of the encoded host label for an Ed25519 public key (32 bytes
/// rounded up to a multiple of 5, divided by 5). A label of any other length
/// cannot be a key.
pub const KEY_HOST_LABEL_LEN: usize = 52;

/// RFC 4648 base32 lowercase, no padding. Lazily initialised because
/// `Specification::encoding` allocates an internal lookup table.
fn base32_lc() -> &'static Encoding {
    use std::sync::OnceLock;
    static ENC: OnceLock<Encoding> = OnceLock::new();
    ENC.get_or_init(|| {
        let mut spec = Specification::new();
        spec.symbols.push_str("abcdefghijklmnopqrstuvwxyz234567");
        spec.encoding().expect("valid base32 lowercase spec")
    })
}

/// Encodes a [`Key`] (the underlying 32-byte Ed25519 public key) as a
/// 52-char lowercase base32 string suitable for use as a DNS label.
pub fn encode_key_to_host_label(key: &Key) -> String {
    base32_lc().encode(key.as_bytes())
}

/// Inverse of [`encode_key_to_host_label`]. Rejects labels that are not
/// exactly [`KEY_HOST_LABEL_LEN`] chars or that contain non-base32-lowercase
/// bytes. The returned error string is suitable to surface to a user.
pub fn decode_host_label_to_key(label: &str) -> Result<Key, String> {
    if label.len() != KEY_HOST_LABEL_LEN {
        return Err(format!(
            "host label is {} chars; expected exactly {}",
            label.len(),
            KEY_HOST_LABEL_LEN
        ));
    }
    let bytes = base32_lc()
        .decode(label.as_bytes())
        .map_err(|err| format!("host label is not valid base32 lowercase: {err}"))?;
    Key::from_bytes(&bytes)
        .map_err(|err| format!("decoded bytes are not a valid Ed25519 key: {err}"))
}

/// Cheap predicate used by the HTTP host dispatcher to decide whether a
/// subdomain is a base32-encoded key (should be decoded into a [`Key`]) or
/// an identity handle (should be looked up via the identity resolver).
///
/// Returns true iff the label is exactly [`KEY_HOST_LABEL_LEN`] characters
/// and every character is in the base32 lowercase alphabet `[a-z2-7]`. An
/// identity handle that fits this exact shape is rejected by
/// `samizdat_common::identity::check_servable_identity`, so the two
/// namespaces do not collide.
pub fn is_base32_key_label(label: &str) -> bool {
    if label.len() != KEY_HOST_LABEL_LEN {
        return false;
    }
    label
        .bytes()
        .all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn sample_key() -> Key {
        // Fixed seed so the test is deterministic; the value of the key is
        // immaterial.
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        Key::from(sk.verifying_key())
    }

    #[test]
    fn round_trip_preserves_key() {
        let key = sample_key();
        let label = encode_key_to_host_label(&key);
        assert_eq!(label.len(), KEY_HOST_LABEL_LEN);
        let back = decode_host_label_to_key(&label).expect("round trip should succeed");
        assert_eq!(back.as_bytes(), key.as_bytes());
    }

    #[test]
    fn label_is_lowercase_base32_alphabet() {
        let key = sample_key();
        let label = encode_key_to_host_label(&key);
        assert!(label.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')));
    }

    #[test]
    fn decode_rejects_wrong_length() {
        let err = decode_host_label_to_key("too-short").expect_err("must reject short label");
        assert!(err.contains("expected exactly 52"));
        let too_long = "a".repeat(KEY_HOST_LABEL_LEN + 1);
        let err = decode_host_label_to_key(&too_long).expect_err("must reject long label");
        assert!(err.contains("expected exactly 52"));
    }

    #[test]
    fn decode_rejects_bad_alphabet() {
        let mut label = encode_key_to_host_label(&sample_key());
        // Replace a character with an out-of-alphabet character.
        label.replace_range(0..1, "1");
        let err = decode_host_label_to_key(&label).expect_err("must reject 1 (not in base32 lc)");
        assert!(err.contains("valid base32 lowercase"));
    }

    #[test]
    fn predicate_accepts_real_key_label() {
        let label = encode_key_to_host_label(&sample_key());
        assert!(is_base32_key_label(&label));
    }

    #[test]
    fn predicate_rejects_short_or_mixed_case_or_out_of_alphabet() {
        assert!(!is_base32_key_label(""));
        assert!(!is_base32_key_label(&"a".repeat(KEY_HOST_LABEL_LEN - 1)));
        let mut label = encode_key_to_host_label(&sample_key());
        label.replace_range(0..1, "A"); // uppercase out of alphabet
        assert!(!is_base32_key_label(&label));
        label.replace_range(0..1, "1"); // digit out of alphabet
        assert!(!is_base32_key_label(&label));
    }
}
