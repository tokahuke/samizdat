//! Cloudflare DNS-01 provider.
//!
//! Talks to the Cloudflare v4 REST API with a scoped API token (the
//! `Authorization: Bearer <token>` shape). Legacy global API keys
//! (`X-Auth-Email` + `X-Auth-Key`) are deliberately not supported; new
//! integrations should never use them.
//!
//! The zone id is resolved lazily on the first call and cached in a
//! `tokio::sync::OnceCell` so subsequent renewals skip the lookup. One
//! `reqwest::Client` is reused across calls.

use async_trait::async_trait;
use serde_derive::Deserialize;
use tokio::sync::OnceCell;

use super::{DnsError, DnsProvider, TxtHandle, http_client};

/// Cloudflare v4 API root. No trailing slash; paths are appended with a
/// leading slash.
const API_BASE: &str = "https://api.cloudflare.com/client/v4";

use crate::dns::util::{ERROR_BODY_LIMIT, truncate_on_boundary};

/// Local wrapper so existing call sites read `truncate(&body)` without
/// re-stating the cap. Delegates to the shared util.
fn truncate(body: &str) -> &str {
    truncate_on_boundary(body, ERROR_BODY_LIMIT)
}

/// Cloudflare DNS-01 provider. Holds the bearer token, a warm HTTP
/// client, and a lazily-populated zone id cache.
pub struct Cloudflare {
    token: String,
    client: reqwest::Client,
    zone_id: OnceCell<String>,
}

impl Cloudflare {
    /// Construct a provider from a Cloudflare API token. The token is
    /// not validated here; the first `set_txt` call (or `check_zone`
    /// from the trait default) is what surfaces auth errors.
    pub fn new(token: String) -> Self {
        Self {
            token,
            client: http_client(),
            zone_id: OnceCell::new(),
        }
    }

    /// Resolve and cache the zone id for `zone`. Subsequent calls reuse
    /// the cached value via `OnceCell::get_or_try_init`.
    async fn zone_id(&self, zone: &str) -> Result<&str, DnsError> {
        let id = self
            .zone_id
            .get_or_try_init(|| async { self.fetch_zone_id(zone).await })
            .await?;
        Ok(id.as_str())
    }

    /// One-shot zone id lookup. Returns the id string parsed out of the
    /// first element of `result`. Empty result or `success: false`
    /// becomes a `DnsError::Provider`.
    async fn fetch_zone_id(&self, zone: &str) -> Result<String, DnsError> {
        let url = format!("{API_BASE}/zones");
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .query(&[("name", zone)])
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(DnsError::Provider(format!(
                "cloudflare zone lookup for {zone} failed with HTTP {status}: {}",
                truncate(&body)
            )));
        }
        let parsed: ZoneListResponse = serde_json::from_str(&body).map_err(|e| {
            DnsError::Provider(format!(
                "cloudflare zone lookup returned unparseable body ({e}): {}",
                truncate(&body)
            ))
        })?;
        if !parsed.success {
            return Err(DnsError::Provider(format!(
                "cloudflare zone lookup for {zone} rejected: {}: {}",
                first_error_message(&parsed.errors),
                truncate(&body)
            )));
        }
        match parsed.result.into_iter().next() {
            Some(z) => Ok(z.id),
            None => Err(DnsError::Provider(format!(
                "cloudflare zone lookup for {zone} returned an empty result; \
                 check that the token has access to the zone"
            ))),
        }
    }
}

#[async_trait]
impl DnsProvider for Cloudflare {
    async fn set_txt(
        &self,
        zone: &str,
        record_name: &str,
        value: &str,
    ) -> Result<TxtHandle, DnsError> {
        // Cloudflare expects the fully qualified name in `name`, not a
        // bare subdomain (this is the inverse of DigitalOcean). Reject
        // mismatches early so callers see a clear error instead of a
        // 400 from the API.
        if record_name != zone && !record_name.ends_with(&format!(".{zone}")) {
            return Err(DnsError::Provider(format!(
                "cloudflare set_txt: record name {record_name} is not within \
                 zone {zone}"
            )));
        }
        let zone_id = self.zone_id(zone).await?;
        let url = format!("{API_BASE}/zones/{zone_id}/dns_records");
        let body = serde_json::json!({
            "type": "TXT",
            "name": record_name,
            "content": value,
            "ttl": 60,
        });
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(DnsError::Provider(format!(
                "cloudflare create TXT for {record_name} failed with HTTP \
                 {status}: {}",
                truncate(&text)
            )));
        }
        let parsed: RecordCreateResponse = serde_json::from_str(&text).map_err(|e| {
            DnsError::Provider(format!(
                "cloudflare create TXT returned unparseable body ({e}): {}",
                truncate(&text)
            ))
        })?;
        if !parsed.success {
            return Err(DnsError::Provider(format!(
                "cloudflare create TXT for {record_name} rejected: {}: {}",
                first_error_message(&parsed.errors),
                truncate(&text)
            )));
        }
        match parsed.result {
            Some(r) => Ok(TxtHandle(r.id)),
            None => Err(DnsError::Provider(format!(
                "cloudflare create TXT for {record_name} returned no record id: {}",
                truncate(&text)
            ))),
        }
    }

    async fn remove_txt(&self, zone: &str, handle: TxtHandle) -> Result<(), DnsError> {
        let zone_id = self.zone_id(zone).await?;
        let record_id = handle.0;
        let url = format!("{API_BASE}/zones/{zone_id}/dns_records/{record_id}");
        let response = self
            .client
            .delete(&url)
            .bearer_auth(&self.token)
            .send()
            .await?;
        let status = response.status();
        // A 404 means someone else already removed the record (manual
        // cleanup, or a previous best-effort delete that beat the
        // current call). Treat the same as success.
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        let text = response.text().await?;
        if !status.is_success() {
            if text.contains("Record not found") {
                return Ok(());
            }
            return Err(DnsError::Provider(format!(
                "cloudflare delete TXT {record_id} failed with HTTP {status}: {}",
                truncate(&text)
            )));
        }
        // Cloudflare returns 200 with success=true and a result object
        // on delete. A success=false 200 still means we should report
        // failure; ditto for an unparseable body. The exception is the
        // "Record not found" string which some edge variants return.
        if text.contains("Record not found") {
            return Ok(());
        }
        let parsed: DeleteResponse = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                return Err(DnsError::Provider(format!(
                    "cloudflare delete TXT returned unparseable body ({e}): {}",
                    truncate(&text)
                )));
            }
        };
        if !parsed.success {
            return Err(DnsError::Provider(format!(
                "cloudflare delete TXT {record_id} rejected: {}: {}",
                first_error_message(&parsed.errors),
                truncate(&text)
            )));
        }
        Ok(())
    }
}

/// Pull the first error message out of a Cloudflare error array, or a
/// placeholder if the array is empty. The caller appends the body
/// snippet separately.
fn first_error_message(errors: &[CfError]) -> String {
    errors
        .first()
        .map(|e| e.message.clone())
        .unwrap_or_else(|| "no error message provided".to_owned())
}

#[derive(Debug, Deserialize)]
struct ZoneListResponse {
    // Cloudflare returns `"result": null` (not an empty array) when
    // `success` is false. Treat null as an empty list so the error path
    // can still read `success` + `errors` to build a useful message.
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    result: Vec<ZoneEntry>,
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
}

fn deserialize_null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    Ok(<Option<T> as serde::Deserialize>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
struct ZoneEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct RecordCreateResponse {
    #[serde(default)]
    result: Option<RecordEntry>,
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
}

#[derive(Debug, Deserialize)]
struct RecordEntry {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DeleteResponse {
    success: bool,
    #[serde(default)]
    errors: Vec<CfError>,
}

#[derive(Debug, Deserialize)]
struct CfError {
    #[serde(default)]
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_zone_list_extracts_first_id() {
        let body = r#"{
            "result": [
                { "id": "abc123zoneid", "name": "example.com" }
            ],
            "success": true,
            "errors": []
        }"#;
        let parsed: ZoneListResponse = serde_json::from_str(body).expect("parses");
        assert!(parsed.success);
        assert_eq!(parsed.result.len(), 1);
        assert_eq!(parsed.result[0].id, "abc123zoneid");
    }

    #[test]
    fn parse_record_create_extracts_id() {
        let body = r#"{
            "result": { "id": "rec_9f8e7d6c", "type": "TXT" },
            "success": true,
            "errors": []
        }"#;
        let parsed: RecordCreateResponse = serde_json::from_str(body).expect("parses");
        assert!(parsed.success);
        assert_eq!(parsed.result.expect("present").id, "rec_9f8e7d6c");
    }

    #[test]
    fn provider_error_carries_cloudflare_message() {
        let body = r#"{
            "result": null,
            "success": false,
            "errors": [
                { "code": 10000, "message": "Authentication error" }
            ]
        }"#;
        // Simulate the rejection branch of `fetch_zone_id` without
        // touching the network: parse, then build the same error
        // string the impl would.
        let parsed: ZoneListResponse = serde_json::from_str(body).expect("parses");
        assert!(!parsed.success);
        let err = DnsError::Provider(format!(
            "cloudflare zone lookup for {zone} rejected: {}: {}",
            first_error_message(&parsed.errors),
            truncate(body),
            zone = "example.com",
        ));
        let DnsError::Provider(msg) = err else {
            panic!("expected DnsError::Provider");
        };
        assert!(
            msg.contains("Authentication error"),
            "missing cloudflare error message in: {msg}",
        );
    }
}

/// `proxy.toml` configuration for the Cloudflare provider.
#[derive(Debug, serde_derive::Deserialize)]
pub struct CloudflareTopology {
    /// Override the default env var name (`CLOUDFLARE_API_TOKEN`).
    #[serde(default)]
    pub token_env: Option<String>,
}

#[typetag::deserialize(name = "cloudflare")]
impl crate::dns::ProviderConfig for CloudflareTopology {
    fn resolve(&self) -> anyhow::Result<Box<dyn crate::dns::DnsProvider>> {
        let var = self
            .token_env
            .clone()
            .unwrap_or_else(|| "CLOUDFLARE_API_TOKEN".to_owned());
        let token = std::env::var(&var).map_err(|_| {
            anyhow::anyhow!("env var {var} is not set; cannot construct Cloudflare provider")
        })?;
        Ok(Box::new(Cloudflare::new(token)))
    }
}
