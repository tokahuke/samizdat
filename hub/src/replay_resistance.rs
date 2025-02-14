//! Tools for replay attacks resistance. This avoids both malicious attacks and unfortunate
//! accidents and is also useful to avoid message amplification attacks to the network.

use std::convert::TryInto;
use tokio::time::{interval, Duration};

use samizdat_common::db::{readonly_tx, writable_tx, Table as _};
use samizdat_common::rpc::{
    EditionAnnouncement, EditionRequest, IdentityRequest, Query, Resolution,
};
use samizdat_common::Hash;

use crate::db::Table;

/// Maximum age for a nonce. 10min allows for some sloppy clocks out there.
const TOLERATED_AGE: i64 = 600;

/// A type that has a nonce (a "number used only once") associated to it.
pub trait Nonce {
    /// Retrieves the nonce associated with this type.
    fn nonce(&self) -> Hash;
}

/// A service for replay attack resistance.
pub struct ReplayResistance;

impl ReplayResistance {
    /// Creates a new ReplayResistance service.
    pub fn new() -> ReplayResistance {
        // Cleanup old nonces. Made deliberately infrequent
        tokio::spawn(async move {
            // Twice would suffice, but thrice is certainty.
            let mut interval = interval(Duration::from_secs(TOLERATED_AGE as u64 * 3));

            loop {
                let now = chrono::Utc::now().timestamp();
                let mut nonces_to_drop = vec![];

                readonly_tx(|tx| {
                    Table::RecentNonces.range(..).for_each(tx, |key, value| {
                        let then =
                            i64::from_be_bytes(value.try_into().expect("bad timestamp from db"));
                        if now - then > 2 * TOLERATED_AGE {
                            // Errors here are leaky, but not a security risk.
                            nonces_to_drop.push(key.to_vec());
                        }

                        None as Option<()>
                    })
                });

                writable_tx(|tx| {
                    for nonce in nonces_to_drop {
                        Table::RecentNonces.delete(tx, nonce);
                    }

                    Ok(())
                })
                .expect("unreachable");

                interval.tick().await;
            }
        });

        ReplayResistance
    }

    /// Checks whether a nonce has been recently seen or not. If the nonce has already
    /// been used, this function returns `Ok(false)` and the caller should reject the
    /// received message. This function returns an error on a database access error.
    ///
    /// Mutability ensures sequential checking of queries, which prevents TOCTOU
    /// when I check against the DB (kinda... must ensure that ReplayResistance is a
    /// *singleton* type too).
    pub fn check<N: Nonce>(&mut self, nonce: &N) -> bool {
        // Is timestamp recent? (timestamp is guaranteed to be generated by the
        // client because hashes!)
        let now = chrono::Utc::now().timestamp();
        let nonce = nonce.nonce();

        // Have I already seen this none before?
        if readonly_tx(|tx| Table::RecentNonces.has(tx, nonce)) {
            return false;
        }

        writable_tx(|tx| {
            Table::RecentNonces.put(tx, nonce.as_ref(), now.to_be_bytes());
            Ok(())
        })
        .expect("infalible");

        true
    }
}

impl Nonce for Query {
    fn nonce(&self) -> Hash {
        self.location_riddle.rand
    }
}

impl Nonce for Resolution {
    fn nonce(&self) -> Hash {
        self.location_message_riddle.rand
    }
}

impl Nonce for EditionRequest {
    fn nonce(&self) -> Hash {
        self.key_riddle.rand
    }
}

impl Nonce for EditionAnnouncement {
    fn nonce(&self) -> Hash {
        self.key_riddle.rand
    }
}

impl Nonce for IdentityRequest {
    fn nonce(&self) -> Hash {
        self.identity_riddle.rand
    }
}
