//! Helpers shared by the HTTP-based providers (e.g. DigitalOcean,
//! Cloudflare, Route53). The script provider does not need these.

/// Cap used when an HTTP provider folds the server's response body into
/// a `DnsError::Provider` string. Long enough to keep the actionable
/// message; short enough that a misbehaving server cannot blow up the
/// log line.
pub(crate) const ERROR_BODY_LIMIT: usize = 512;

/// Truncate `s` to at most `limit` bytes without splitting a multibyte
/// UTF-8 sequence. Walks back to the previous char boundary if `limit`
/// itself lands inside a character.
pub(crate) fn truncate_on_boundary(s: &str, limit: usize) -> &str {
    if s.len() <= limit {
        return s;
    }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_under_limit_is_noop() {
        assert_eq!(truncate_on_boundary("hello", 10), "hello");
    }

    #[test]
    fn truncate_at_ascii_boundary() {
        assert_eq!(truncate_on_boundary("abcdef", 3), "abc");
    }

    #[test]
    fn truncate_walks_back_off_multibyte() {
        // 'é' is two bytes in UTF-8; truncating at byte 1 must walk back
        // to byte 0 rather than splitting the codepoint.
        let s = "é";
        assert_eq!(s.len(), 2);
        assert_eq!(truncate_on_boundary(s, 1), "");
    }
}
