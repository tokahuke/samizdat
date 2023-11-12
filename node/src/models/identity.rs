//! Identities are human-readable names associated with series public keys. This is
//! currently a proposal that guarantees a very weak consensus based on sheer raw
//! proof-of-work.

use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::fmt::Display;
use std::str::FromStr;

use samizdat_common::Hash;
use samizdat_common::{pow::ProofOfWork, Riddle};

use crate::db;
use crate::db::Table;

use super::{Droppable, SeriesRef};

/// Minimum 1GHash for an identity to be valid.
const MINIMUM_WORK_DONE: f64 = 1e9;

/// A reference to an identity.
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
    /// Creates an identity reference from binary data.
    pub fn from_bytes(bytes: &[u8]) -> Result<IdentityRef, crate::Error> {
        String::from_utf8_lossy(bytes).parse()
    }

    /// Gets the hash of this identity.
    pub fn hash(&self) -> Hash {
        Hash::from_bytes(&self.handle)
    }

    /// Gets the handle (i.e., human-readable name) of this identity.
    pub fn handle(&self) -> &str {
        &self.handle
    }

    /// Retrieves the identity from the database.
    pub fn get(&self) -> Result<Option<Identity>, crate::Error> {
        if let Some(serialized) = db().get_cf(Table::Identities.get(), self.hash())? {
            let identity = bincode::deserialize(&serialized)?;
            Ok(Some(identity))
        } else {
            Ok(None)
        }
    }
}

/// A Samizdat identity claim.
#[derive(Debug, Serialize, Deserialize)]
pub struct Identity {
    /// The identity reference, containing the handle (i.e., human-readable name) of this
    /// identity.
    pub identity: IdentityRef,
    /// The series associated with this identity, containing the series public key.
    pub series: SeriesRef,
    /// A proof of work for the association between handle and public key. For the same
    /// identity reference, identities with greater proof-of-work should superseed
    /// identities with smaller proof-of-work.
    pub proof: ProofOfWork,
}

impl Droppable for Identity {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        batch.delete_cf(Table::Identities.get(), self.identity().hash());
        Ok(())
    }
}

impl Identity {
    /// Runs through the database trying to find an identity that fits to the supplied
    /// content riddle. Returns `Ok(None)` if no matching item is found.
    pub fn find(riddle: &Riddle) -> Result<Option<Identity>, crate::Error> {
        let it = db().iterator_cf(Table::Identities.get(), IteratorMode::Start);

        for item in it {
            let (key, value) = item?;
            match Hash::try_from(&*key) {
                Ok(hash) => {
                    if riddle.resolves(&hash) {
                        match bincode::deserialize(&value) {
                            Ok(identity) => return Ok(Some(identity)),
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

        Ok(None)
    }

    /// Retrieves an identity from the database for a given identity reference.
    pub fn get(identity: &IdentityRef) -> Result<Option<Identity>, crate::Error> {
        Ok(db()
            .get_cf(Table::Identities.get(), identity.hash())?
            .map(|value| bincode::deserialize(&value))
            .transpose()?)
    }

    /// Lists all the identities currently in the database.
    pub fn get_all() -> Result<Vec<Identity>, crate::Error> {
        db().iterator_cf(Table::Identities.get(), IteratorMode::Start)
            .map(|item| {
                let (_, value) = item?;
                Ok(bincode::deserialize(&value)?)
            })
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    /// Inserts the current identity in the database using the supplied [`WriteBatch`].
    pub fn insert(&self, batch: &mut WriteBatch) {
        batch.put_cf(
            Table::Identities.get(),
            self.identity().hash(),
            bincode::serialize(&self).expect("can serialize"),
        );
    }

    /// Returns the identity reference for this identity.
    pub fn identity(&self) -> &IdentityRef {
        &self.identity
    }

    /// Returns the series reference for this identity.
    pub fn series(self) -> SeriesRef {
        self.series
    }

    /// Checks whether this identity is valid, that is
    ///
    /// 1. If the handle is valid.
    /// 2. If the proof-of-work matches the provided handle and public key.
    /// 3. If the work done is at least the minimum amount required.
    pub fn is_valid(&self) -> bool {
        let expected_information = self
            .identity()
            .hash()
            .rehash(&self.series.public_key().hash());
        let valid_identity = self.identity().handle().parse::<IdentityRef>().is_ok();
        let correct_pow = self.proof.information == expected_information;

        correct_pow && valid_identity && self.proof.work_done() > MINIMUM_WORK_DONE
    }

    /// Gets the work done by the proof-of-work for this identity.
    pub fn work_done(&self) -> f64 {
        self.proof.work_done()
    }

    /// Creates a new identity.
    ///
    /// # Note
    ///
    /// This function can be very computationally intense. Besides, it is single-threaded.
    /// To create real-world identities, use the CLI instead.
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

    /// Creates a new identity and inserts it into the database.
    ///
    /// # Note
    ///
    /// This function can be very computationally intense. Besides, it is single-threaded.
    /// To create real-world identities, use the CLI instead.
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
