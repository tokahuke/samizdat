use std::convert::TryInto;
use tokio::time::{interval, Duration};

use samizdat_common::rpc::{Query, Resolution};
use samizdat_common::Hash;

use crate::db;
use crate::db::Table;

/// 10min allows for some sloppy clocks out there.
const TOLERATED_AGE: i64 = 600;

pub trait Nonce {
    fn nonce(&self) -> Hash;
    fn timestamp(&self) -> i64;
}

pub struct ReplayResistance;

impl ReplayResistance {
    pub fn new() -> ReplayResistance {
        // Cleanup old nonces. Made deliberately infrequent
        tokio::spawn(async move {
            // Twice would suffice, but thrice is certainty.
            let mut interval = interval(Duration::from_secs(TOLERATED_AGE as u64 * 3));

            loop {
                let now = chrono::Utc::now().timestamp();

                for (key, val) in
                    db().iterator_cf(Table::RecentNonces.get(), rocksdb::IteratorMode::Start)
                {
                    let then =
                        i64::from_be_bytes((&*val).try_into().expect("bad timestamp from db"));
                    if now - then > 2 * TOLERATED_AGE {
                        // Errors here are leaky, but not a security risk.
                        db().delete_cf(Table::RecentNonces.get(), key).ok();
                    }
                }

                interval.tick().await;
            }
        });

        ReplayResistance
    }

    /// Mutability ensures sequential checking of queries, which prevents TOCTOU
    /// when I check against the DB (kinda... must ensure *singleton*).
    pub fn check<N: Nonce>(&mut self, nonce: &N) -> Result<bool, crate::Error> {
        // Is timestamp recent? (timestamp is guaranteed to be generated by the
        // client because hashes!)
        let now = chrono::Utc::now().timestamp();
        let then = nonce.timestamp();
        let nonce = nonce.nonce();

        if (now - then).abs() > TOLERATED_AGE {
            return Ok(false);
        }

        // Have I already seen this none before?
        if db().get_cf(Table::RecentNonces.get(), nonce)?.is_some() {
            return Ok(false);
        }

        db().put_cf(Table::RecentNonces.get(), nonce, &then.to_be_bytes())?;

        Ok(true)
    }
}

impl Nonce for Query {
    fn nonce(&self) -> Hash {
        self.content_riddle.hash
    }

    fn timestamp(&self) -> i64 {
        self.content_riddle.timestamp
    }
}

impl Nonce for Resolution {
    fn nonce(&self) -> Hash {
        self.content_riddle.hash
    }

    fn timestamp(&self) -> i64 {
        self.content_riddle.timestamp
    }
}
