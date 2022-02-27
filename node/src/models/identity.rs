use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::fmt::Display;
use std::str::FromStr;

use samizdat_common::Hash;
use samizdat_common::{pow::ProofOfWork, Riddle};

use crate::db;
use crate::db::Table;

use super::{Dropable, SeriesRef};

/// Minimum 1GHash for an identity to be valid.
const MINIMUM_WORK_DONE: f64 = 1e9;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct IdentityRef {
    /// A valid identity handle.
    handle: String,
}

impl FromStr for IdentityRef {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            invalid @ ("" | "~" | "." | "..") => {
                Err(format!("Identity handle cannot be `{invalid}`").into())
            }
            s if s.starts_with('_') => {
                Err(format!("Identity handle `{s}` starting with `_`").into())
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

impl IdentityRef {
    pub fn from_bytes(bytes: &[u8]) -> Result<IdentityRef, crate::Error> {
        String::from_utf8_lossy(bytes).parse()
    }

    pub fn hash(&self) -> Hash {
        Hash::hash(&self.handle)
    }

    pub fn handle(&self) -> &str {
        &self.handle
    }

    pub fn get(&self) -> Result<Option<Identity>, crate::Error> {
        if let Some(serialized) = db().get_cf(Table::Identities.get(), self.hash())? {
            let identity = bincode::deserialize(&serialized)?;
            Ok(Some(identity))
        } else {
            Ok(None)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Identity {
    pub identity: IdentityRef,
    pub series: SeriesRef,
    pub proof: ProofOfWork,
}

impl Dropable for Identity {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        batch.delete_cf(Table::Identities.get(), self.identity().hash());
        Ok(())
    }
}

impl Identity {
    pub fn find(riddle: &Riddle) -> Option<Identity> {
        let it = db().iterator_cf(Table::Identities.get(), IteratorMode::Start);

        for (key, value) in it {
            match IdentityRef::from_bytes(&key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        match bincode::deserialize(&value) {
                            Ok(identity) => return Some(identity),
                            Err(err) => {
                                log::warn!("{err}");
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    log::warn!("{err}");
                    continue;
                }
            }
        }

        None
    }

    pub fn get_all() -> Result<Vec<Identity>, crate::Error> {
        db().iterator_cf(Table::Identities.get(), IteratorMode::Start)
            .map(|(_, value)| Ok(bincode::deserialize(&value)?))
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    pub fn insert(&self, batch: &mut WriteBatch) {
        batch.put_cf(
            Table::Identities.get(),
            self.identity().hash(),
            bincode::serialize(&self).expect("can serialize"),
        );
    }

    pub fn identity(&self) -> &IdentityRef {
        &self.identity
    }

    pub fn series(self) -> SeriesRef {
        self.series
    }

    pub fn is_valid(&self) -> bool {
        let expected_information = self
            .identity()
            .hash()
            .rehash(&self.series.public_key().hash());
        let valid_identity = self.identity().handle().parse::<IdentityRef>().is_ok();
        let correct_pow = self.proof.information == expected_information;

        correct_pow && valid_identity && self.proof.work_done() > MINIMUM_WORK_DONE
    }

    pub fn work_done(&self) -> f64 {
        self.proof.work_done()
    }

    fn forge(identity: IdentityRef, series: SeriesRef, work_target: f64) -> Identity {
        let mut proof = ProofOfWork::new(identity.hash().rehash(&series.public_key().hash()));

        while proof.work_done() < f64::max(work_target, MINIMUM_WORK_DONE) {
            let mut new_proof = proof.clone();
            new_proof.solution = Hash::rand();

            if new_proof.work_done() > proof.work_done() {
                proof = new_proof;
            }
        }

        Identity {
            identity,
            series,
            proof,
        }
    }

    pub fn create(
        identity: IdentityRef,
        series: SeriesRef,
        work_target: f64,
    ) -> Result<Identity, crate::Error> {
        // Contrary to other entities in the DB, this entity may have a lot of collisions.
        if db()
            .get_cf(Table::Identities.get(), identity.hash())?
            .is_some()
        {
            return Err(crate::Error::Message(format!(
                "Identity `{identity}` already exists. Delete it first!"
            )));
        }

        let identity = Self::forge(identity, series, work_target);
        let mut batch = WriteBatch::default();
        identity.insert(&mut batch);

        db().write(batch)?;

        Ok(identity)
    }
}
