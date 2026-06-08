//! DigitalOcean DNS-01 provider. Talks to the v2 API at
//! `https://api.digitalocean.com/v2/domains/<zone>/records` with a bearer
//! token. The provider serialises requests by hand to avoid pulling in
//! `serde_json` as a direct dependency of `samizdat-proxy`; the response
//! shape we care about is a single integer field, which we extract with
//! a small ad-hoc parser.

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use std::fmt::Write;

use super::{http_client, DnsError, DnsProvider, TxtHandle};

const API_BASE: &str = "https://api.digitalocean.com/v2";

use crate::dns::util::{truncate_on_boundary as truncate, ERROR_BODY_LIMIT};

/// DigitalOcean DNS-01 provider. Owns a bearer token and a reqwest
/// client shared across renewals via `http_client`.
pub struct DigitalOcean {
    token: String,
    http: Client,
}

impl DigitalOcean {
    /// Build a provider from a personal access token. The token must
    /// carry the `domain:create`/`domain:delete` scopes for any zone the
    /// proxy needs to publish records into.
    pub fn new(token: String) -> Self {
        DigitalOcean {
            token,
            http: http_client(),
        }
    }
}

#[async_trait]
impl DnsProvider for DigitalOcean {
    async fn set_txt(
        &self,
        zone: &str,
        record_name: &str,
        value: &str,
    ) -> Result<TxtHandle, DnsError> {
        let subdomain = derive_subdomain(record_name, zone)?;
        let body = format!(
            "{{\"type\":\"TXT\",\"name\":{name},\"data\":{data},\"ttl\":60}}",
            name = json_string(&subdomain),
            data = json_string(value),
        );
        let url = format!("{API_BASE}/domains/{zone}/records");
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(body)
            .send()
            .await?;

        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(DnsError::Provider(format!(
                "digitalocean: status={} body={}",
                status.as_u16(),
                truncate(&text, ERROR_BODY_LIMIT),
            )));
        }

        let id = extract_record_id(&text)?;
        Ok(TxtHandle(id.to_string()))
    }

    async fn remove_txt(&self, zone: &str, handle: TxtHandle) -> Result<(), DnsError> {
        let url = format!("{API_BASE}/domains/{zone}/records/{}", handle.0);
        let response = self
            .http
            .delete(&url)
            .bearer_auth(&self.token)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = response.status();
        if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        if status.is_success() {
            return Ok(());
        }
        let text = response.text().await.unwrap_or_default();
        Err(DnsError::Provider(format!(
            "digitalocean: status={} body={}",
            status.as_u16(),
            truncate(&text, ERROR_BODY_LIMIT),
        )))
    }
}

/// Strip the trailing `.<zone>` from `record_name` to produce the
/// subdomain DO expects in the request body. The bare zone maps to
/// `"@"`. A mismatched suffix is a configuration bug; surface it as a
/// provider error so the cert manager logs it loudly.
fn derive_subdomain(record_name: &str, zone: &str) -> Result<String, DnsError> {
    if record_name == zone {
        return Ok("@".to_owned());
    }
    let suffix = format!(".{zone}");
    if let Some(prefix) = record_name.strip_suffix(&suffix) {
        if prefix.is_empty() {
            return Ok("@".to_owned());
        }
        return Ok(prefix.to_owned());
    }
    Err(DnsError::Provider(format!(
        "digitalocean: record name {record_name:?} is not inside zone {zone:?}"
    )))
}

/// Pull `domain_record.id` (an integer) out of a DO create-record
/// response. Hand-written rather than via `serde_json` because the
/// crate is not a direct dependency of `samizdat-proxy`. The response
/// shape is stable: a single top-level object with a `domain_record`
/// member whose first int field is `id`.
fn extract_record_id(body: &str) -> Result<i64, DnsError> {
    // Find the "domain_record" key.
    let key = "\"domain_record\"";
    let key_at = body.find(key).ok_or_else(|| {
        DnsError::Provider(format!(
            "digitalocean: response missing `domain_record`: body={}",
            truncate(body, ERROR_BODY_LIMIT),
        ))
    })?;
    let after_key = &body[key_at + key.len()..];

    // Inside the `domain_record` object, locate the `"id"` field.
    let id_key = "\"id\"";
    let id_at = after_key.find(id_key).ok_or_else(|| {
        DnsError::Provider(format!(
            "digitalocean: response missing `domain_record.id`: body={}",
            truncate(body, ERROR_BODY_LIMIT),
        ))
    })?;
    let after_id = &after_key[id_at + id_key.len()..];

    // Skip whitespace and the colon, then read the integer literal.
    let mut chars = after_id.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() || c == ':' {
            chars.next();
        } else {
            break;
        }
    }
    let mut digits = String::new();
    if let Some(&c) = chars.peek() {
        if c == '-' {
            digits.push(c);
            chars.next();
        }
    }
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            digits.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if digits.is_empty() || digits == "-" {
        return Err(DnsError::Provider(format!(
            "digitalocean: response `domain_record.id` is not an integer: body={}",
            truncate(body, ERROR_BODY_LIMIT),
        )));
    }
    digits.parse::<i64>().map_err(|_| {
        DnsError::Provider(format!(
            "digitalocean: response `domain_record.id` out of range: body={}",
            truncate(body, ERROR_BODY_LIMIT),
        ))
    })
}

/// Escape `s` as a JSON string literal (including surrounding quotes).
/// Handles the subset of escapes that can appear in TXT challenge
/// payloads and DNS labels: quote, backslash, control chars below 0x20.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_subdomain_strips_zone() {
        let got =
            derive_subdomain("_acme-challenge.proxy.example.com", "example.com").unwrap();
        assert_eq!(got, "_acme-challenge.proxy");
    }

    #[test]
    fn derive_subdomain_bare_zone_is_at() {
        let got = derive_subdomain("example.com", "example.com").unwrap();
        assert_eq!(got, "@");
    }

    #[test]
    fn derive_subdomain_mismatch_errors() {
        let err = derive_subdomain("_acme-challenge.proxy.other.com", "example.com")
            .unwrap_err();
        match err {
            DnsError::Provider(msg) => {
                assert!(msg.contains("not inside zone"), "got: {msg}");
            }
            _ => panic!("expected DnsError::Provider, got {err:?}"),
        }
    }

    #[test]
    fn extract_record_id_round_trip() {
        let body = r#"{
            "domain_record": {
                "id": 1234567,
                "type": "TXT",
                "name": "_acme-challenge.proxy",
                "data": "abc",
                "priority": null,
                "port": null,
                "ttl": 60,
                "weight": null,
                "flags": null,
                "tag": null
            }
        }"#;
        let id = extract_record_id(body).unwrap();
        assert_eq!(id, 1_234_567);
    }

    #[test]
    fn extract_record_id_missing_id_errors() {
        let body = r#"{ "domain_record": { "type": "TXT", "name": "x", "data": "y" } }"#;
        let err = extract_record_id(body).unwrap_err();
        match err {
            DnsError::Provider(msg) => {
                assert!(
                    msg.contains("domain_record.id"),
                    "expected message to mention domain_record.id, got: {msg}"
                );
            }
            _ => panic!("expected DnsError::Provider, got {err:?}"),
        }
    }

    #[test]
    fn json_string_escapes_specials() {
        assert_eq!(json_string("plain"), "\"plain\"");
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
    }
}

/// `proxy.toml` configuration for the DigitalOcean provider.
#[derive(Debug, serde_derive::Deserialize)]
pub struct DigitalOceanTopology {
    /// Override the default env var name (`DIGITALOCEAN_TOKEN`).
    #[serde(default)]
    pub token_env: Option<String>,
}

#[typetag::deserialize(name = "digitalocean")]
impl crate::dns::ProviderConfig for DigitalOceanTopology {
    fn resolve(&self) -> anyhow::Result<Box<dyn crate::dns::DnsProvider>> {
        let var = self
            .token_env
            .clone()
            .unwrap_or_else(|| "DIGITALOCEAN_TOKEN".to_owned());
        let token = std::env::var(&var)
            .map_err(|_| anyhow::anyhow!("env var {var} is not set; cannot construct DO provider"))?;
        Ok(Box::new(DigitalOcean::new(token)))
    }
}
