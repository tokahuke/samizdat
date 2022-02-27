use std::fmt::Display;
use std::str::FromStr;
use std::thread;
use tabled::Tabled;

use samizdat_common::{pow::ProofOfWork, Hash, Key};

use crate::api::{self, post_identity, PostIdentityRequest};
use crate::util::{Metric, Unit};

use super::show_table;

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
    n_iters: usize,
) -> Result<(), anyhow::Error> {
    let identity: IdentityRef = identity_handle.parse()?;
    let series_owner = api::get_series_owner(&series_name).await?;
    let key = Key::from(series_owner.keypair.public);

    let information = Hash::hash(&identity.handle).rehash(&key.hash());

    let handles = (0..num_cpus::get())
        .map(|thread_id| {
            let proof_of_work = ProofOfWork::new(information);

            thread::spawn(move || {
                let mut local_best = proof_of_work.clone();
                let mut rng = samizdat_common::csprng();

                for _ in 0..n_iters {
                    let mut new_try = proof_of_work.clone();
                    new_try.solution = Hash::rand_with(&mut rng);

                    if new_try.work_done() > local_best.work_done() {
                        local_best = new_try;
                        log::debug!("New best @ {thread_id}: {local_best:#?}");
                    }
                }

                local_best
            })
        })
        .collect::<Vec<_>>();

    let proof = handles
        .into_iter()
        .map(|handle| handle.join().expect("thread panicked"))
        .max_by_key(|pow| pow.work_done() as usize)
        .expect("num_cpus >= 1");

    log::debug!("Final PoW: {proof:?}");

    post_identity(PostIdentityRequest {
        identity: &identity_handle,
        series: &key.to_string(),
        proof,
    })
    .await?;

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
    let information = Hash::hash(&identity_handle).rehash(&series.hash());
    let proof = ProofOfWork { information, solution};

    post_identity(PostIdentityRequest {
        identity: &identity_handle,
        series: &series.to_string(),
        proof,
    })
    .await?;

    Ok(())
}
