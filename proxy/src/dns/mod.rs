//! DNS-01 ACME challenge providers.
//!
//! Each implementation talks to one DNS provider's HTTP API directly via
//! `reqwest`; no provider SDKs. The wildcard cert manager
//! (`crate::wildcard`) calls into this trait once per renewal cycle to
//! place and then remove the `_acme-challenge` TXT record that Let's
//! Encrypt validates against.

use std::time::Duration;

use serde_derive::Deserialize;

mod aws_sigv4;
pub mod cloudflare;
pub mod digitalocean;
pub mod route53;
pub mod script;
pub(crate) mod util;

/// Opaque in-process token returned by `set_txt` and handed back to
/// `remove_txt`. Providers store the provider-specific record identifier
/// they need to address a single record: DigitalOcean's numeric record
/// id, Cloudflare's string record id, Route53's record value (deletion
/// in Route53 requires the byte-identical record body). The handle is
/// never persisted to disk; orphaned records are tolerated.
#[derive(Debug, Clone)]
pub struct TxtHandle(pub String);

/// Errors a provider can surface. Deliberately minimal: the cert
/// manager treats transport errors and provider-side rejections the
/// same way (log and retry with backoff). Grow this enum when a
/// concrete branch needs distinguishing.
#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("dns provider transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("dns provider rejected request: {0}")]
    Provider(String),
}

/// The contract every DNS provider implements. `async fn` in trait is
/// fine here because the trait is only ever dyn-dispatched behind an
/// `Arc<dyn DnsProvider + Send + Sync>` owned by the cert manager; the
/// desugaring cost is paid once per renewal cycle (every ~60 days),
/// not per request.
#[async_trait::async_trait]
pub trait DnsProvider: Send + Sync {
    /// Create a TXT record at `record_name` in `zone` with the given
    /// `value`. The implementation chooses the lowest TTL the provider
    /// permits (all three of DO, Cloudflare, Route53 accept 60s); the
    /// record is short-lived. Returns a provider-specific handle that
    /// the manager passes back to `remove_txt`.
    async fn set_txt(
        &self,
        zone: &str,
        record_name: &str,
        value: &str,
    ) -> Result<TxtHandle, DnsError>;

    /// Delete the record `handle` previously returned by `set_txt`.
    /// Best-effort: the cert manager logs and ignores any error here.
    async fn remove_txt(&self, zone: &str, handle: TxtHandle) -> Result<(), DnsError>;

    /// Startup smoke test. Default impl tries a set + remove cycle on a
    /// sentinel name; refusing to boot on failure is preferable to
    /// discovering the misconfiguration 60 days later when the cert is
    /// about to expire. Implementations override only if they have a
    /// cheaper smoke test; none of the built-ins do.
    async fn check_zone(&self, zone: &str) -> Result<(), DnsError> {
        let name = format!("_samizdat-preflight.{zone}");
        let value = format!("samizdat-preflight-{}", rand::random::<u64>());
        let handle = self.set_txt(zone, &name, &value).await?;
        self.remove_txt(zone, handle).await
    }
}

/// Top-level `[dns]` block in `proxy.toml`. The provider is dispatched
/// via `typetag` and flattened, so the operator writes one flat table:
///
/// ```toml
/// [dns]
/// zone = "hubfederation.com"
/// wildcard_root = "hubfederation.com"
/// provider = "digitalocean"
/// token_env = "DIGITALOCEAN_TOKEN"
/// ```
#[derive(Debug, Deserialize)]
pub struct DnsTopology {
    /// Apex of the DNS zone the credentials can write inside (for
    /// `hubfederation.com`, this is typically also `hubfederation.com`).
    pub zone: String,
    /// The name the wildcard cert covers. Cert SANs are
    /// `<wildcard_root>` and `*.<wildcard_root>`.
    pub wildcard_root: String,
    /// Provider-specific config; deserialized polymorphically via
    /// `typetag` on the `type = "..."` discriminator.
    #[serde(flatten)]
    pub provider: Box<dyn ProviderConfig>,
}

/// Per-provider configuration. Each implementation lives in its own
/// module and registers itself with a `#[typetag::deserialize(name =
/// "...")]` attribute; the operator's `proxy.toml` selects between
/// them via the `provider = "..."` field inside `[dns]`.
#[typetag::deserialize(tag = "provider")]
pub trait ProviderConfig: Send + Sync + std::fmt::Debug {
    /// Read any required environment variables and construct the
    /// concrete `DnsProvider`.
    fn resolve(&self) -> anyhow::Result<Box<dyn DnsProvider>>;
}

/// Shared reqwest client builder for the HTTP providers (DO,
/// Cloudflare, Route53). The same client gets reused across calls so
/// connections are kept warm across renewals.
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("samizdat-proxy/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("can build reqwest client")
}
