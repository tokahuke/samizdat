//! Wildcard cert lifecycle: ACME DNS-01 issuance and renewal for the
//! single cert that covers `<wildcard_root>` plus `*.<wildcard_root>`.
//!
//! The cert manager owns the configured [`DnsProvider`], drives the ACME
//! state machine via `instant-acme`, and exposes a hot-swappable
//! [`rustls::server::ResolvesServerCert`] that the TLS listener picks up.
//! The renewal task wakes every 12 hours, checks expiry, and runs a full
//! issuance if the existing cert is within 30 days of `not_after`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use arc_swap::ArcSwapOption;
use chrono::{DateTime, Utc};
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, NewAccount, NewOrder, OrderStatus,
};
use rcgen::{CertificateParams, DistinguishedName, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde_derive::{Deserialize, Serialize};
use tokio::fs;
use tokio::time::sleep;
use tracing::{error, info, warn};

use crate::dns::DnsProvider;

/// File name on disk for the issued cert chain (PEM).
const CERT_FILENAME: &str = "cert.pem";
/// File name on disk for the cert's private key (PEM, PKCS#8).
const KEY_FILENAME: &str = "key.pem";
/// File name on disk for the cert's metadata (issued and expiry).
const META_FILENAME: &str = "meta.json";
/// File name on disk for the ACME account credentials.
const ACCOUNT_FILENAME: &str = "account.json";

/// Renewal kicks in when the cert is within this many days of expiry.
const RENEWAL_WINDOW: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// The renewal loop wakes this often to re-check.
const RENEWAL_TICK: Duration = Duration::from_secs(12 * 60 * 60);
/// Maximum time the manager will wait for the ACME server to validate.
const ORDER_POLL_TIMEOUT: Duration = Duration::from_secs(120);

/// Persisted alongside the cert PEMs so the manager can decide whether
/// to renew without parsing the cert.
#[derive(Debug, Serialize, Deserialize)]
struct CertMeta {
    issued_at: DateTime<Utc>,
    not_after: DateTime<Utc>,
}

/// Owns the wildcard cert and drives its renewal.
pub struct WildcardCertManager {
    provider: Box<dyn DnsProvider>,
    zone: String,
    wildcard_root: String,
    cert_dir: PathBuf,
    acme_directory_url: String,
    contact_email: String,
    snapshot: Arc<ArcSwapOption<CertifiedKey>>,
}

impl WildcardCertManager {
    /// Build a manager with the provided DNS-01 driver and on-disk
    /// directory for cached cert state.
    pub fn new(
        provider: Box<dyn DnsProvider>,
        zone: String,
        wildcard_root: String,
        cert_dir: PathBuf,
        acme_directory_url: String,
        contact_email: String,
    ) -> Self {
        Self {
            provider,
            zone,
            wildcard_root,
            cert_dir,
            acme_directory_url,
            contact_email,
            snapshot: Arc::new(ArcSwapOption::empty()),
        }
    }

    /// Hand back the rustls resolver that observes the live cert
    /// snapshot. Cheap to clone; share across the TLS server's
    /// `ServerConfig`.
    pub fn resolver(&self) -> Arc<WildcardResolver> {
        Arc::new(WildcardResolver {
            snapshot: Arc::clone(&self.snapshot),
        })
    }

    /// Smoke-test the DNS provider against the configured zone before
    /// the proxy starts accepting connections. Refusing to boot on
    /// failure is preferable to discovering the misconfiguration 60
    /// days later when the cert is about to expire.
    pub async fn check_zone(&self) -> anyhow::Result<()> {
        self.provider
            .check_zone(&self.zone)
            .await
            .map_err(|err| anyhow::anyhow!("dns provider preflight failed: {err}"))
    }

    /// Block until the on-disk cert is loaded into the snapshot. If no
    /// cert exists yet, kick off an issuance and wait for it. After
    /// this returns, the TLS server can serve handshakes; the renewal
    /// task takes over for subsequent cycles.
    pub async fn prime(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.cert_dir)
            .await
            .with_context(|| format!("creating cert dir {:?}", self.cert_dir))?;

        if self.try_load_disk_cert().await? {
            info!("loaded wildcard cert from disk");
            return Ok(());
        }

        info!("no wildcard cert on disk; issuing one now");
        self.run_issuance().await?;
        Ok(())
    }

    /// Run forever, renewing the cert when it approaches expiry. Spawn
    /// this into the runtime; never returns under normal operation.
    pub async fn run_renewal(self: Arc<Self>) {
        loop {
            sleep(RENEWAL_TICK).await;
            let _self_ref = &*self;
            match self.read_meta().await {
                Ok(Some(meta)) => {
                    let remaining = meta.not_after.signed_duration_since(Utc::now());
                    let renewal_window = chrono::Duration::from_std(RENEWAL_WINDOW)
                        .expect("renewal window fits chrono duration");
                    if remaining > renewal_window {
                        continue;
                    }
                    if remaining < chrono::Duration::days(7) {
                        error!(
                            "wildcard cert expires in {} days, renewal pending",
                            remaining.num_days()
                        );
                    } else {
                        info!(
                            "wildcard cert expires in {} days, renewing now",
                            remaining.num_days()
                        );
                    }
                }
                Ok(None) => {
                    warn!("no wildcard cert metadata on disk; issuing now");
                }
                Err(err) => {
                    error!("could not read wildcard cert metadata: {err}; renewing anyway");
                }
            }

            if let Err(err) = self.run_issuance().await {
                error!("wildcard cert renewal failed: {err}; will retry next tick");
            }
        }
    }

    /// Try to read the cached cert PEMs from disk into the snapshot.
    /// Returns Ok(true) on success, Ok(false) if either file is
    /// missing, Err on parse failure.
    async fn try_load_disk_cert(&self) -> anyhow::Result<bool> {
        let cert_path = self.cert_dir.join(CERT_FILENAME);
        let key_path = self.cert_dir.join(KEY_FILENAME);
        if !cert_path.exists() || !key_path.exists() {
            return Ok(false);
        }

        let cert_pem = fs::read(&cert_path).await.context("reading cert.pem")?;
        let key_pem = fs::read(&key_path).await.context("reading key.pem")?;
        let certified_key = certified_key_from_pem(&cert_pem, &key_pem)
            .context("building CertifiedKey from cached PEMs")?;
        self.snapshot.store(Some(Arc::new(certified_key)));
        Ok(true)
    }

    /// Read the on-disk metadata file if it exists. Used by the renewal
    /// loop to decide whether to renew.
    async fn read_meta(&self) -> anyhow::Result<Option<CertMeta>> {
        let path = self.cert_dir.join(META_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).await.context("reading meta.json")?;
        let meta: CertMeta = serde_json::from_slice(&bytes).context("parsing meta.json")?;
        Ok(Some(meta))
    }

    /// Run one full ACME issuance: account bootstrap, order, DNS-01
    /// challenge, finalize, persist PEMs, update snapshot.
    async fn run_issuance(&self) -> anyhow::Result<()> {
        let account = self.load_or_create_account().await?;
        let bare = self.wildcard_root.clone();
        let wildcard = format!("*.{}", self.wildcard_root);
        let identifiers = vec![
            Identifier::Dns(bare.clone()),
            Identifier::Dns(wildcard.clone()),
        ];
        let mut order = account
            .new_order(&NewOrder::new(&identifiers))
            .await
            .context("creating ACME order")?;

        let mut placed_records = Vec::new();
        let challenge_name = format!("_acme-challenge.{}", self.wildcard_root);
        let mut authorizations = order.authorizations();
        while let Some(authz) = authorizations.next().await {
            let mut authz = authz.context("fetching ACME authorization")?;
            if !matches!(authz.status, AuthorizationStatus::Pending) {
                continue;
            }
            let mut challenge = authz
                .challenge(ChallengeType::Dns01)
                .context("ACME authorization has no dns-01 challenge")?;
            let value = challenge.key_authorization().dns_value();
            let handle = self
                .provider
                .set_txt(&self.zone, &challenge_name, &value)
                .await
                .map_err(|err| anyhow::anyhow!("dns provider set_txt failed: {err}"))?;
            placed_records.push(handle);
            challenge
                .set_ready()
                .await
                .context("marking ACME challenge ready")?;
        }

        let final_status = wait_until_ready(&mut order).await?;
        if final_status != OrderStatus::Ready && final_status != OrderStatus::Valid {
            cleanup_records(self.provider.as_ref(), &self.zone, placed_records).await;
            anyhow::bail!("ACME order did not reach Ready (final status {final_status:?})");
        }

        // Build a CSR with both names as SANs.
        let key_pair =
            KeyPair::generate().context("generating fresh keypair for wildcard cert")?;
        let mut params = CertificateParams::new(vec![bare.clone(), wildcard.clone()])
            .context("building CSR params")?;
        params.distinguished_name = DistinguishedName::new();
        let csr = params
            .serialize_request(&key_pair)
            .context("serializing CSR")?;

        order
            .finalize_csr(csr.der().as_ref())
            .await
            .context("ACME finalize")?;

        let cert_chain_pem = loop {
            match order
                .certificate()
                .await
                .context("downloading ACME certificate")?
            {
                Some(pem) => break pem,
                None => sleep(Duration::from_secs(2)).await,
            }
        };

        let key_pem = key_pair.serialize_pem();
        self.persist_artifacts(cert_chain_pem.as_bytes(), key_pem.as_bytes())
            .await?;

        let certified_key = certified_key_from_pem(cert_chain_pem.as_bytes(), key_pem.as_bytes())
            .context("loading freshly-issued PEMs into rustls")?;
        self.snapshot.store(Some(Arc::new(certified_key)));

        cleanup_records(self.provider.as_ref(), &self.zone, placed_records).await;

        info!("wildcard cert issued and live");
        Ok(())
    }

    /// Persist the cert + key PEMs and the metadata file. Files are
    /// written next to each other then renamed into place so a crash
    /// mid-write never serves a half-rewritten cert.
    async fn persist_artifacts(&self, cert_pem: &[u8], key_pem: &[u8]) -> anyhow::Result<()> {
        let now = Utc::now();
        let meta = CertMeta {
            issued_at: now,
            not_after: now + chrono::Duration::days(90),
        };
        let meta_bytes = serde_json::to_vec_pretty(&meta).context("serializing meta.json")?;

        atomic_write(self.cert_dir.join(CERT_FILENAME), cert_pem).await?;
        atomic_write(self.cert_dir.join(KEY_FILENAME), key_pem).await?;
        atomic_write(self.cert_dir.join(META_FILENAME), &meta_bytes).await?;
        Ok(())
    }

    /// Load the ACME account from disk if it exists, otherwise register
    /// a fresh one and persist its credentials. Re-using the account
    /// across renewals keeps the proxy under Let's Encrypt's per-account
    /// rate limits and avoids one round-trip per renewal.
    async fn load_or_create_account(&self) -> anyhow::Result<Account> {
        let path = self.cert_dir.join(ACCOUNT_FILENAME);
        if path.exists() {
            let bytes = fs::read(&path).await.context("reading account.json")?;
            let creds: instant_acme::AccountCredentials =
                serde_json::from_slice(&bytes).context("parsing account.json")?;
            let account = Account::builder()
                .context("building ACME account builder")?
                .from_credentials(creds)
                .await
                .context("rehydrating ACME account from credentials")?;
            return Ok(account);
        }

        let contact = format!("mailto:{}", self.contact_email);
        let (account, creds) = Account::builder()
            .context("building ACME account builder")?
            .create(
                &NewAccount {
                    contact: &[contact.as_str()],
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                self.acme_directory_url.clone(),
                None,
            )
            .await
            .context("creating ACME account")?;

        let bytes = serde_json::to_vec_pretty(&creds).context("serializing account.json")?;
        atomic_write(path, &bytes).await?;
        Ok(account)
    }
}

/// Rustls cert resolver that snapshots into the live cert. Returns
/// `None` until the manager has primed its first cert, which causes
/// rustls to refuse the handshake; in normal operation the manager
/// primes synchronously before the listener binds.
pub struct WildcardResolver {
    snapshot: Arc<ArcSwapOption<CertifiedKey>>,
}

impl ResolvesServerCert for WildcardResolver {
    fn resolve(&self, _hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        self.snapshot.load_full()
    }
}

impl std::fmt::Debug for WildcardResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WildcardResolver {{ ... }}")
    }
}

/// Poll the ACME order until it reaches Ready or Invalid, or the
/// timeout elapses.
async fn wait_until_ready(order: &mut instant_acme::Order) -> anyhow::Result<OrderStatus> {
    let deadline = tokio::time::Instant::now() + ORDER_POLL_TIMEOUT;
    let mut backoff = Duration::from_secs(1);
    loop {
        let state = order
            .refresh()
            .await
            .context("refreshing ACME order state")?;
        match state.status {
            OrderStatus::Ready | OrderStatus::Valid => return Ok(state.status),
            OrderStatus::Invalid => return Ok(OrderStatus::Invalid),
            OrderStatus::Pending | OrderStatus::Processing => {
                if tokio::time::Instant::now() >= deadline {
                    return Ok(state.status);
                }
                sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(10));
            }
        }
    }
}

/// Best-effort cleanup of the TXT records placed during issuance.
/// Failures are logged and ignored; orphaned records are harmless to
/// future renewals because each renewal asks for a fresh challenge
/// token.
async fn cleanup_records(
    provider: &dyn DnsProvider,
    zone: &str,
    records: Vec<crate::dns::TxtHandle>,
) {
    for handle in records.into_iter().rev() {
        if let Err(err) = provider.remove_txt(zone, handle).await {
            warn!("dns provider remove_txt failed (orphan tolerated): {err}");
        }
    }
}

/// Parse the cert chain PEM and the key PEM into a rustls
/// `CertifiedKey` ready to serve.
fn certified_key_from_pem(cert_pem: &[u8], key_pem: &[u8]) -> anyhow::Result<CertifiedKey> {
    let cert_chain: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut std::io::BufReader::new(cert_pem))
            .collect::<Result<_, _>>()
            .context("parsing cert chain")?;
    if cert_chain.is_empty() {
        anyhow::bail!("cert PEM contained no certificates");
    }

    let key_der: PrivateKeyDer<'static> =
        rustls_pemfile::private_key(&mut std::io::BufReader::new(key_pem))
            .context("parsing private key")?
            .context("private key PEM contained no key")?;

    let signing_key = rustls::crypto::ring::sign::any_supported_type(&key_der)
        .context("constructing rustls signing key")?;
    Ok(CertifiedKey::new(cert_chain, signing_key))
}

/// Write `bytes` to `path` atomically: write to a sibling temp file
/// then rename over. Crash-safe on POSIX; on Windows the rename is
/// also atomic for the same volume.
async fn atomic_write(path: PathBuf, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .context("atomic write target has no parent dir")?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("write")
    ));
    fs::write(&tmp, bytes)
        .await
        .with_context(|| format!("writing temp file {:?}", tmp))?;
    fs::rename(&tmp, &path)
        .await
        .with_context(|| format!("renaming temp into {:?}", path))?;
    Ok(())
}

/// HTTPS entrypoint for the wildcard-cert path. Mirrors
/// `crate::acme::serve` but drives ACME DNS-01 through the configured
/// provider instead of HTTP-01.
pub async fn serve(
    dns: &crate::dns::DnsTopology,
    owner: &str,
    acme_directory: &str,
    cert_dir: PathBuf,
    addr: std::net::SocketAddr,
    http_port: u16,
    app: axum::Router,
) -> anyhow::Result<()> {
    use std::sync::Arc;

    let provider = dns
        .provider
        .resolve()
        .context("resolving DNS provider for wildcard cert")?;
    let manager = Arc::new(WildcardCertManager::new(
        provider,
        dns.zone.clone(),
        dns.wildcard_root.clone(),
        cert_dir,
        acme_directory.to_owned(),
        owner.to_owned(),
    ));
    manager.check_zone().await?;
    manager.prime().await?;

    let resolver = manager.resolver();
    tokio::spawn({
        let manager = Arc::clone(&manager);
        async move { manager.run_renewal().await }
    });

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(resolver);
    let rustls_config =
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config));

    let mut http_addr = addr;
    http_addr.set_port(http_port);
    let http_listener = tokio::net::TcpListener::bind(http_addr).await?;

    let redirector = crate::acme::redirect_to_https_for(&dns.wildcard_root, addr);
    let (https_outcome, http_outcome) = tokio::join!(
        axum_server::bind_rustls(addr, rustls_config).serve(app.into_make_service()),
        axum::serve(http_listener, redirector.into_make_service()),
    );
    http_outcome.context("serving the HTTP redirector")?;
    https_outcome.context("serving the HTTPS server")?;
    Ok(())
}

/// Helper exposed for the `wildcard.toml` schema (the
/// `[acme] directory` field would have a default).
pub fn default_acme_directory() -> String {
    "https://acme-v02.api.letsencrypt.org/directory".to_owned()
}

#[allow(dead_code)]
fn _dirty(_p: &Path) {}
