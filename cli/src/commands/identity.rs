use std::fmt::Display;
use std::str::FromStr;
use std::thread;
use tabled::Tabled;
use tokio::sync::mpsc;

use samizdat_common::{pow::ProofOfWork, Hash, Key};

use crate::api::{self, post_identity, PostIdentityRequest};
use crate::util::{Metric, Unit};

use super::show_table;

/// Minimum 1GHash for an identity to be valid.
const MINIMUM_WORK_DONE: f64 = 1e9;

#[derive(Debug, Clone)]
pub struct IdentityRef {
    /// A valid identity handle.
    handle: String,
}

impl FromStr for IdentityRef {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            invalid @ ("" | "~" | "." | "..") => {
                anyhow::bail!("Identity handle cannot be `{invalid}`")
            }
            s if s.starts_with('_') => {
                anyhow::bail!("Identity handle `{s}` starting with `_`")
            }
            s => Ok(IdentityRef {
                handle: s.to_owned(),
            }),
        }
    }
}

impl Display for IdentityRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.handle)
    }
}

pub async fn forge(
    identity_handle: String,
    series_name: String,
    n_iters: Option<usize>,
) -> Result<(), anyhow::Error> {
    let identity: IdentityRef = identity_handle.parse()?;
    let series_owner = api::get_series_owner(&series_name).await?;
    let key = Key::from(series_owner.keypair.public);

    let information = Hash::from_bytes(&identity.handle).rehash(&key.hash());

    // Sender task to update Samizdat Node of the current best PoW:
    let (send, mut recv) = mpsc::unbounded_channel();
    let sender_identity_handle = identity_handle.clone();
    let sender_series = key.clone();
    tokio::spawn(async move {
        while let Some(proof) = recv.recv().await {
            let post_outcome = post_identity(PostIdentityRequest {
                identity: &sender_identity_handle,
                series: &sender_series.to_string(),
                proof: ProofOfWork::clone(&proof),
            })
            .await;

            if let Err(error) = post_outcome {
                log::error!("Failed to send proof-of-work {proof:?}: {error}");
            }
        }
    });

    // Threads calculating PoWs:
    let n_threads = (num_cpus::get() as f32 / 0.8).ceil() as usize;
    let handles = (0..n_threads)
        .map(|thread_id| {
            let proof_of_work = ProofOfWork::new(information);
            let send = send.clone();

            thread::spawn(move || {
                let mut local_best = proof_of_work.clone();
                let mut rng = samizdat_common::csprng();

                // TODO: Ooops! if 32-bit, this is a bit low...
                for _ in 0..n_iters.unwrap_or(usize::MAX) {
                    let mut new_try = proof_of_work.clone();
                    new_try.solution = Hash::rand_with(&mut rng);

                    if new_try.work_done() > local_best.work_done() {
                        local_best = new_try;
                        log::debug!("New best @ {thread_id}: {local_best:#?}");

                        if local_best.work_done() > MINIMUM_WORK_DONE {
                            send.send(local_best.clone())
                                .expect("request sender panicked");
                        }
                    }
                }

                local_best
            })
        })
        .collect::<Vec<_>>();

    // Drop last sender. This will stop the sender task when the last thread resumes.
    let _ = send;

    // Calculate best (only to display):
    let proof = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread panicked"))
        .max_by_key(|pow| pow.work_done() as usize)
        .expect("num_cpus >= 1");

    log::debug!("Final PoW: {proof:?}");

    Ok(())
}

pub async fn ls(identity_handle: Option<String>) -> Result<(), anyhow::Error> {
    async fn ls_identity(_identity_handle: String) -> Result<(), anyhow::Error> {
        todo!()
    }

    async fn ls_all() -> Result<(), anyhow::Error> {
        let identities = api::get_all_identities().await?;

        struct HashPower;
        impl Unit for HashPower {
            const SYMBOL: &'static str = "H";
        }

        #[derive(Tabled)]
        struct Row {
            handle: String,
            series: String,
            pow_solution: Hash,
            work_done: Metric<HashPower>,
        }

        show_table(identities.into_iter().map(|identity| Row {
            handle: identity.identity.handle,
            series: identity.series.public_key.to_string(),
            pow_solution: identity.proof.solution,
            work_done: HashPower::value(identity.proof.work_done()),
        }));

        Ok(())
    }

    if let Some(identity_handle) = identity_handle {
        ls_identity(identity_handle).await
    } else {
        ls_all().await
    }
}

pub async fn import(
    identity_handle: String,
    series: Key,
    solution: Hash,
) -> Result<(), anyhow::Error> {
    let information = Hash::from_bytes(&identity_handle).rehash(&series.hash());
    let proof = ProofOfWork {
        information,
        solution,
    };

    post_identity(PostIdentityRequest {
        identity: &identity_handle,
        series: &series.to_string(),
        proof,
    })
    .await?;

    Ok(())
}
