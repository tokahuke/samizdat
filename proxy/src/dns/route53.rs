//! AWS Route53 DNS-01 provider.
//!
//! Talks to the Route53 XML API at `https://route53.amazonaws.com/`
//! directly. Every request is signed with SigV4 via the local
//! `aws_sigv4` module; no `aws-*` crate dependency. The responses are
//! small fixed-shape XML documents, and we hand-extract the one or two
//! fields we need rather than pull in an XML parser.
//!
//! The Route53 record-set change flow has two steps:
//!   1. POST a `ChangeResourceRecordSetsRequest` with action UPSERT or
//!      DELETE, get back a change id and a status of `PENDING`.
//!   2. Poll `GET /change/<id>` until the status flips to `INSYNC`
//!      across the Route53 fleet, then the record is visible to
//!      external resolvers including Let's Encrypt's.
//!
//! The poll is bounded; on timeout we warn and return success, because
//! Let's Encrypt's external DNS probe (with its own retries) is the
//! authoritative check, and the cert manager will retry the whole
//! issuance if validation actually fails.

use std::collections::HashMap;
use std::fmt::Write;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use tokio::sync::Mutex;
use tokio::time::sleep;

use super::aws_sigv4::{sign, SigV4Request};
use super::{http_client, DnsError, DnsProvider, TxtHandle};

/// Route53 service host. SigV4 scope uses `route53` as the service
/// name; the region in the scope is whatever the caller configured
/// (Route53 itself is global; the region only affects the signing
/// scope string).
const HOST: &str = "route53.amazonaws.com";

/// API base path. Route53's URL prefix is the API version stamp.
const API_VERSION: &str = "/2013-04-01";

/// SigV4 service name. Always `route53`.
const SERVICE: &str = "route53";

use crate::dns::util::{truncate_on_boundary as truncate, ERROR_BODY_LIMIT};

/// Maximum wall-clock to wait for a change to propagate to INSYNC.
const PROPAGATION_TIMEOUT: Duration = Duration::from_secs(60);

/// Interval between `GET /change/<id>` polls while waiting for INSYNC.
const PROPAGATION_POLL: Duration = Duration::from_secs(2);

/// Separator used to pack `(record_name, value)` into a single opaque
/// `TxtHandle`. NUL is fine because neither DNS record names nor TXT
/// challenge values are allowed to contain it; using it keeps the
/// handle opaque to the rest of the system while the impl can split
/// the two halves back out.
const HANDLE_SEP: char = '\u{0000}';

/// AWS Route53 DNS-01 provider. Holds the static credentials and the
/// shared reqwest client. Zone ids are looked up on first use per
/// `set_txt` call and cached in memory so repeat renewals against the
/// same zone do not pay the lookup round-trip.
pub struct Route53 {
    region: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    client: Client,
    zone_ids: Mutex<HashMap<String, String>>,
}

impl Route53 {
    /// Build a provider from explicit credentials. The session token is
    /// only set when the operator is using STS temporary credentials;
    /// long-lived IAM user keys leave it `None`.
    pub fn new(
        region: String,
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
    ) -> Self {
        Route53 {
            region,
            access_key_id,
            secret_access_key,
            session_token,
            client: http_client(),
            zone_ids: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve a zone name (e.g. `example.com`) to its Route53 hosted
    /// zone id (e.g. `Z1234ABCDEF`). Cached per canonical name with the
    /// trailing dot. A miss does a single `ListHostedZonesByName` call
    /// and stores the result; failures are not negatively cached.
    async fn zone_id(&self, zone: &str) -> Result<String, DnsError> {
        let canonical = format!("{}.", zone.trim_end_matches('.'));
        if let Some(id) = self.zone_ids.lock().await.get(&canonical).cloned() {
            return Ok(id);
        }

        let path = format!("{API_VERSION}/hostedzonesbyname");
        // Route53 wants the dnsname value as a bare DNS label sequence;
        // the trailing dot is required so the lookup matches the
        // canonical form. The value is plain ASCII for any real zone
        // name; percent-encode it conservatively all the same.
        let query = format!("dnsname={}", encode_query_value(&canonical));
        let body = self
            .request("GET", &path, &query, b"")
            .await?;
        let id = extract_first_capture(&body, "<Id>/hostedzone/", "</Id>").ok_or_else(|| {
            DnsError::Provider(format!(
                "route53: hostedzonesbyname response missing Id: body={}",
                truncate(&body, ERROR_BODY_LIMIT),
            ))
        })?;

        self.zone_ids
            .lock()
            .await
            .insert(canonical, id.clone());
        Ok(id)
    }

    /// Submit a signed HTTP request and return the response body as a
    /// `String`. Non-2xx responses (other than the caller-handled 404
    /// path) turn into `DnsError::Provider`. The body cap on errors is
    /// applied at the call site for messages that include the body.
    async fn request(
        &self,
        method: &str,
        path: &str,
        query: &str,
        body: &[u8],
    ) -> Result<String, DnsError> {
        let signed = sign(SigV4Request {
            method,
            host: HOST,
            path,
            query,
            body,
            region: &self.region,
            service: SERVICE,
            access_key_id: &self.access_key_id,
            secret_access_key: &self.secret_access_key,
            session_token: self.session_token.as_deref(),
        });

        let url = if query.is_empty() {
            format!("https://{HOST}{path}")
        } else {
            format!("https://{HOST}{path}?{query}")
        };
        let mut builder = self
            .client
            .request(method.parse().expect("static method string"), &url)
            .header("Host", HOST)
            .header("X-Amz-Date", &signed.x_amz_date)
            .header("X-Amz-Content-Sha256", &signed.x_amz_content_sha256)
            .header("Authorization", &signed.authorization);
        if let Some(token) = &signed.x_amz_security_token {
            builder = builder.header("X-Amz-Security-Token", token);
        }
        if method == "POST" {
            builder = builder.header("Content-Type", "application/xml");
        }
        if !body.is_empty() {
            builder = builder.body(body.to_vec());
        }

        let response = builder.send().await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(DnsError::Provider(format!(
                "route53: status={} body={}",
                status.as_u16(),
                truncate(&text, ERROR_BODY_LIMIT),
            )));
        }
        Ok(text)
    }

    /// POST a record-set change with the given action. Returns the
    /// change id parsed out of the response. The XML body is built by
    /// `build_change_body`; both `set_txt` and `remove_txt` use it with
    /// different action verbs.
    async fn submit_change(
        &self,
        zone_id: &str,
        action: &str,
        record_name: &str,
        value: &str,
    ) -> Result<String, DnsError> {
        let body = build_change_body(action, record_name, value);
        let path = format!("{API_VERSION}/hostedzone/{zone_id}/rrset");
        let response = self
            .request("POST", &path, "", body.as_bytes())
            .await?;
        extract_first_capture(&response, "<Id>/change/", "</Id>").ok_or_else(|| {
            DnsError::Provider(format!(
                "route53: change response missing Id: body={}",
                truncate(&response, ERROR_BODY_LIMIT),
            ))
        })
    }

    /// Poll `GET /change/<id>` until the status is `INSYNC` or the
    /// timeout fires. A timeout is logged at warn level and otherwise
    /// ignored: the ACME validator will retry the DNS check, and if
    /// the record really is not visible the whole issuance attempt
    /// fails noisily from the cert manager's perspective.
    async fn wait_for_insync(&self, change_id: &str) {
        let path = format!("{API_VERSION}/change/{change_id}");
        let deadline = tokio::time::Instant::now() + PROPAGATION_TIMEOUT;
        loop {
            match self.request("GET", &path, "", b"").await {
                Ok(body) => {
                    if body.contains("<Status>INSYNC</Status>") {
                        return;
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        change_id = %change_id,
                        error = %format_dns_error(&err),
                        "route53: change status poll failed; will retry",
                    );
                }
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    change_id = %change_id,
                    "route53: change did not reach INSYNC within \
                     {PROPAGATION_TIMEOUT:?}; proceeding and trusting \
                     the ACME validator's own retry loop",
                );
                return;
            }
            sleep(PROPAGATION_POLL).await;
        }
    }
}

#[async_trait]
impl DnsProvider for Route53 {
    async fn set_txt(
        &self,
        zone: &str,
        record_name: &str,
        value: &str,
    ) -> Result<TxtHandle, DnsError> {
        let zone_id = self.zone_id(zone).await?;
        let canonical_name = ensure_trailing_dot(record_name);
        let change_id = self
            .submit_change(&zone_id, "UPSERT", &canonical_name, value)
            .await?;
        self.wait_for_insync(&change_id).await;
        // Handle packs the canonical record name and the unescaped TXT
        // value so `remove_txt` can rebuild the exact XML body. NUL
        // separator keeps the encoding opaque.
        Ok(TxtHandle(format!("{canonical_name}{HANDLE_SEP}{value}")))
    }

    async fn remove_txt(&self, zone: &str, handle: TxtHandle) -> Result<(), DnsError> {
        let zone_id = self.zone_id(zone).await?;
        let (record_name, value) = split_handle(&handle.0).ok_or_else(|| {
            DnsError::Provider(
                "route53: malformed TxtHandle; expected `<name>\\x00<value>`".to_owned(),
            )
        })?;

        let body = build_change_body("DELETE", record_name, value);
        let path = format!("{API_VERSION}/hostedzone/{zone_id}/rrset");
        let signed = sign(SigV4Request {
            method: "POST",
            host: HOST,
            path: &path,
            query: "",
            body: body.as_bytes(),
            region: &self.region,
            service: SERVICE,
            access_key_id: &self.access_key_id,
            secret_access_key: &self.secret_access_key,
            session_token: self.session_token.as_deref(),
        });
        let url = format!("https://{HOST}{path}");
        let mut builder = self
            .client
            .post(&url)
            .header("Host", HOST)
            .header("X-Amz-Date", &signed.x_amz_date)
            .header("X-Amz-Content-Sha256", &signed.x_amz_content_sha256)
            .header("Authorization", &signed.authorization)
            .header("Content-Type", "application/xml")
            .body(body.into_bytes());
        if let Some(token) = &signed.x_amz_security_token {
            builder = builder.header("X-Amz-Security-Token", token);
        }
        let response = builder.send().await?;
        let status = response.status();
        if status.is_success() || status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        let text = response.text().await.unwrap_or_default();
        Err(DnsError::Provider(format!(
            "route53: status={} body={}",
            status.as_u16(),
            truncate(&text, ERROR_BODY_LIMIT),
        )))
    }
}

/// Render `DnsError` for the warn log without consuming it. The trait
/// already implements `Display`; this exists so the `tracing` field
/// formatter can pick it up by reference.
fn format_dns_error(err: &DnsError) -> String {
    err.to_string()
}

/// Build the XML body for a `ChangeResourceRecordSetsRequest`. The
/// caller supplies the action (`UPSERT` or `DELETE`), the fully
/// qualified record name (with trailing dot), and the unquoted value.
/// The function wraps the value in double quotes (mandated by the
/// Route53 wire format for TXT records) and escapes any `"` or `\`
/// inside it as `\"` / `\\`.
fn build_change_body(action: &str, record_name: &str, value: &str) -> String {
    let escaped = escape_txt_value(value);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <ChangeResourceRecordSetsRequest xmlns=\"https://route53.amazonaws.com/doc/2013-04-01/\">\n  \
           <ChangeBatch>\n    \
             <Changes>\n      \
               <Change>\n        \
                 <Action>{action}</Action>\n        \
                 <ResourceRecordSet>\n          \
                   <Name>{record_name}</Name>\n          \
                   <Type>TXT</Type>\n          \
                   <TTL>60</TTL>\n          \
                   <ResourceRecords>\n            \
                     <ResourceRecord>\n              \
                       <Value>\"{escaped}\"</Value>\n            \
                     </ResourceRecord>\n          \
                   </ResourceRecords>\n        \
                 </ResourceRecordSet>\n      \
               </Change>\n    \
             </Changes>\n  \
           </ChangeBatch>\n\
         </ChangeResourceRecordSetsRequest>\n",
    )
}

/// Escape a TXT record value for inclusion inside the quoted form
/// Route53 expects. Backslashes and double quotes are the only two
/// characters that need escaping; everything else is passed through
/// verbatim. ACME challenge tokens are base64url and never contain
/// either, but the cert manager is allowed to renew records for
/// non-ACME uses too, so the escaping is implemented properly.
fn escape_txt_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out
}

/// Append a `.` to the record name if it is not already terminated.
/// Route53 rejects record names that lack the canonical trailing dot;
/// callers tend to pass FQDNs without one.
fn ensure_trailing_dot(name: &str) -> String {
    if name.ends_with('.') {
        name.to_owned()
    } else {
        format!("{name}.")
    }
}

/// Split a `TxtHandle` payload back into `(record_name, value)`. The
/// separator is NUL; if it is missing the handle is malformed and the
/// caller turns this into a `DnsError::Provider`.
fn split_handle(packed: &str) -> Option<(&str, &str)> {
    packed.split_once(HANDLE_SEP)
}

/// Find the first occurrence of `prefix`, then return the substring
/// between it and the next `suffix`. Used to pull the hosted zone id
/// out of `<Id>/hostedzone/Z123</Id>` and the change id out of
/// `<Id>/change/C123</Id>` without taking a dependency on `quick-xml`
/// for what is a stable two-line shape.
fn extract_first_capture(body: &str, prefix: &str, suffix: &str) -> Option<String> {
    let start = body.find(prefix)? + prefix.len();
    let tail = &body[start..];
    let end = tail.find(suffix)?;
    Some(tail[..end].to_owned())
}

/// Percent-encode a query-string value per RFC 3986. The Route53
/// hostedzonesbyname call only ever takes a DNS name plus the
/// mandatory trailing dot, so the encoder largely no-ops; it is
/// implemented to keep us correct against pathological zone names.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        let unreserved = b.is_ascii_alphanumeric()
            || b == b'-'
            || b == b'.'
            || b == b'_'
            || b == b'~';
        if unreserved {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{:02X}", b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_zone_id_from_list_response() {
        let body = r#"<?xml version="1.0"?>
            <ListHostedZonesByNameResponse>
              <HostedZones>
                <HostedZone>
                  <Id>/hostedzone/Z1234ABCDEF</Id>
                  <Name>example.com.</Name>
                  <CallerReference>r</CallerReference>
                </HostedZone>
              </HostedZones>
              <DNSName>example.com.</DNSName>
              <IsTruncated>false</IsTruncated>
              <MaxItems>100</MaxItems>
            </ListHostedZonesByNameResponse>"#;
        let id = extract_first_capture(body, "<Id>/hostedzone/", "</Id>").unwrap();
        assert_eq!(id, "Z1234ABCDEF");
    }

    #[test]
    fn extracts_change_id_from_change_response() {
        let body = r#"<?xml version="1.0"?>
            <ChangeResourceRecordSetsResponse>
              <ChangeInfo>
                <Id>/change/C1234567</Id>
                <Status>PENDING</Status>
                <SubmittedAt>2025-01-01T00:00:00Z</SubmittedAt>
              </ChangeInfo>
            </ChangeResourceRecordSetsResponse>"#;
        let id = extract_first_capture(body, "<Id>/change/", "</Id>").unwrap();
        assert_eq!(id, "C1234567");
    }

    #[test]
    fn extract_returns_none_when_missing() {
        assert!(extract_first_capture("<no/>", "<Id>/change/", "</Id>").is_none());
    }

    #[test]
    fn ensure_trailing_dot_is_idempotent() {
        assert_eq!(ensure_trailing_dot("a.b.example.com"), "a.b.example.com.");
        assert_eq!(ensure_trailing_dot("a.b.example.com."), "a.b.example.com.");
    }

    #[test]
    fn escape_txt_value_handles_quotes_and_backslashes() {
        assert_eq!(escape_txt_value("abc"), "abc");
        assert_eq!(escape_txt_value("a\"b"), "a\\\"b");
        assert_eq!(escape_txt_value("a\\b"), "a\\\\b");
        assert_eq!(escape_txt_value("a\"b\\c"), "a\\\"b\\\\c");
    }

    #[test]
    fn split_handle_round_trip() {
        let packed = format!("_acme-challenge.proxy.example.com.{HANDLE_SEP}some-value");
        let (name, value) = split_handle(&packed).unwrap();
        assert_eq!(name, "_acme-challenge.proxy.example.com.");
        assert_eq!(value, "some-value");
    }

    #[test]
    fn split_handle_rejects_unseparated() {
        assert!(split_handle("nothing-here").is_none());
    }

    /// Pin the exact XML body the signer is asked to commit to. Any
    /// drift in indentation, attribute order, or the quoting of the
    /// `<Value>` field would change the signature and the wire bytes
    /// at once; locking it down here prevents silent regressions in
    /// either direction.
    #[test]
    fn build_change_body_is_byte_identical() {
        let body = build_change_body(
            "UPSERT",
            "_acme-challenge.proxy.example.com.",
            "challenge-token-abc",
        );
        let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <ChangeResourceRecordSetsRequest xmlns=\"https://route53.amazonaws.com/doc/2013-04-01/\">\n  \
              <ChangeBatch>\n    \
                <Changes>\n      \
                  <Change>\n        \
                    <Action>UPSERT</Action>\n        \
                    <ResourceRecordSet>\n          \
                      <Name>_acme-challenge.proxy.example.com.</Name>\n          \
                      <Type>TXT</Type>\n          \
                      <TTL>60</TTL>\n          \
                      <ResourceRecords>\n            \
                        <ResourceRecord>\n              \
                          <Value>\"challenge-token-abc\"</Value>\n            \
                        </ResourceRecord>\n          \
                      </ResourceRecords>\n        \
                    </ResourceRecordSet>\n      \
                  </Change>\n    \
                </Changes>\n  \
              </ChangeBatch>\n\
            </ChangeResourceRecordSetsRequest>\n";
        assert_eq!(body, expected);
    }

    #[test]
    fn build_change_body_delete_uses_delete_action() {
        let body =
            build_change_body("DELETE", "_acme-challenge.example.com.", "v");
        assert!(body.contains("<Action>DELETE</Action>"));
        assert!(body.contains("<Value>\"v\"</Value>"));
    }

    #[test]
    fn encode_query_value_passes_through_dns_names() {
        assert_eq!(encode_query_value("example.com."), "example.com.");
        assert_eq!(encode_query_value("foo bar"), "foo%20bar");
    }
}

/// `proxy.toml` configuration for the Route53 provider.
#[derive(Debug, serde_derive::Deserialize)]
pub struct Route53Topology {
    /// AWS region. Route53 itself is global, but SigV4 requires a region
    /// in the signing scope; the default `us-east-1` works everywhere.
    #[serde(default = "default_route53_region")]
    pub region: String,
    /// Override the default env var name (`AWS_ACCESS_KEY_ID`).
    #[serde(default)]
    pub access_key_id_env: Option<String>,
    /// Override the default env var name (`AWS_SECRET_ACCESS_KEY`).
    #[serde(default)]
    pub secret_access_key_env: Option<String>,
    /// Override the default env var name (`AWS_SESSION_TOKEN`). The
    /// session token itself is optional and only used by STS temp creds;
    /// absent is fine.
    #[serde(default)]
    pub session_token_env: Option<String>,
}

fn default_route53_region() -> String {
    "us-east-1".to_owned()
}

#[typetag::deserialize(name = "route53")]
impl crate::dns::ProviderConfig for Route53Topology {
    fn resolve(&self) -> anyhow::Result<Box<dyn crate::dns::DnsProvider>> {
        let id_var = self
            .access_key_id_env
            .clone()
            .unwrap_or_else(|| "AWS_ACCESS_KEY_ID".to_owned());
        let secret_var = self
            .secret_access_key_env
            .clone()
            .unwrap_or_else(|| "AWS_SECRET_ACCESS_KEY".to_owned());
        let session_var = self
            .session_token_env
            .clone()
            .unwrap_or_else(|| "AWS_SESSION_TOKEN".to_owned());
        let access_key_id = std::env::var(&id_var).map_err(|_| {
            anyhow::anyhow!("env var {id_var} is not set; cannot construct Route53 provider")
        })?;
        let secret_access_key = std::env::var(&secret_var).map_err(|_| {
            anyhow::anyhow!(
                "env var {secret_var} is not set; cannot construct Route53 provider"
            )
        })?;
        let session_token = std::env::var(&session_var).ok();
        Ok(Box::new(Route53::new(
            self.region.clone(),
            access_key_id,
            secret_access_key,
            session_token,
        )))
    }
}
