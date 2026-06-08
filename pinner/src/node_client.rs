//! Thin reqwest client around the local samizdat-node's `/_subscriptions/`
//! admin endpoints. The pinner reads the bearer token from
//! `~/.samizdat/access-token` (created by the node on first run) and uses
//! it for every request.

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use samizdat_common::Key;
use serde_derive::Serialize;

static CLIENT: OnceLock<NodeClient> = OnceLock::new();

pub fn init(node_base: &str) -> Result<(), anyhow::Error> {
    let token = read_admin_token().context("reading ~/.samizdat/access-token")?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    CLIENT
        .set(NodeClient {
            base: node_base.trim_end_matches('/').to_string(),
            token,
            http,
        })
        .map_err(|_| anyhow::anyhow!("node_client double-init"))?;
    Ok(())
}

pub fn get() -> &'static NodeClient {
    CLIENT.get().expect("node_client not initialized")
}

pub struct NodeClient {
    base: String,
    token: String,
    http: reqwest::Client,
}

#[derive(Serialize)]
struct PostSubscription<'a> {
    public_key: &'a str,
    kind: &'static str,
}

impl NodeClient {
    /// Idempotent: a 4xx because the subscription already exists is folded
    /// into success, so the pinner can blindly re-call this on every pin
    /// request without worrying about prior state.
    pub async fn add_subscription(&self, key: &Key) -> Result<(), anyhow::Error> {
        let url = format!("{}/_subscriptions/", self.base);
        let body = PostSubscription {
            public_key: &key.to_string(),
            kind: "FullInventory",
        };
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await?;
        if resp.status().is_success() {
            return Ok(());
        }
        // The node's POST returns an error on duplicate insert; treat any
        // 4xx as "already there or not our problem to retry." 5xx still
        // bubbles up.
        if resp.status().is_client_error() {
            tracing::debug!(
                "add_subscription({key}) returned {}: treated as already-present",
                resp.status()
            );
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("add_subscription({key}) failed: {status}: {text}")
    }

    /// Triggers the node's bookmark-releasing DELETE (the change in
    /// `node/src/http/subscriptions.rs`). Idempotent on the node side.
    pub async fn drop_subscription(&self, key: &Key) -> Result<(), anyhow::Error> {
        let url = format!("{}/_subscriptions/{}", self.base, key);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&self.token)
            .send()
            .await?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("drop_subscription({key}) failed: {status}: {text}")
    }
}

fn read_admin_token() -> Result<String, anyhow::Error> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let path = format!("{home}/.samizdat/access-token");
    let token = std::fs::read_to_string(&path)
        .with_context(|| format!("reading admin token at {path}"))?;
    Ok(token.trim().to_string())
}
