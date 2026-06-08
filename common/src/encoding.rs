//! Canonical wire encoding for samizdat entity identifiers.
//!
//! Every public entity id ([`crate::Key`], [`crate::Hash`]) renders the
//! same way everywhere: CLI args, URLs, JSON, logs. The encoding is
//! RFC 4648 base32 lowercase without padding (alphabet `[a-z2-7]`),
//! chosen because it fits the DNS label alphabet so an id can be
//! pasted into a subdomain verbatim. 28-byte hash -> 45 chars;
//! 32-byte key -> 52 chars.

use data_encoding::{Encoding, Specification};

/// RFC 4648 base32 lowercase, no padding. Lazily initialised because
/// `Specification::encoding` allocates an internal lookup table.
pub fn base32_lc() -> &'static Encoding {
    use std::sync::OnceLock;
    static ENC: OnceLock<Encoding> = OnceLock::new();
    ENC.get_or_init(|| {
        let mut spec = Specification::new();
        spec.symbols.push_str("abcdefghijklmnopqrstuvwxyz234567");
        spec.encoding().expect("valid base32 lowercase spec")
    })
}
