use serde_derive::{Deserialize, Serialize};
use std::net::IpAddr;

use samizdat_common::db::{writable_tx, Table as _, WritableTx};

use crate::db::Table;

#[derive(Debug, Serialize, Deserialize)]
pub struct BlacklistedIp {
    address: IpAddr,
    since: chrono::DateTime<chrono::Utc>,
}

impl BlacklistedIp {
    pub fn new(address: IpAddr) -> BlacklistedIp {
        BlacklistedIp {
            address,
            since: chrono::Utc::now(),
        }
    }

    pub fn get(address: IpAddr) -> Result<Option<BlacklistedIp>, crate::Error> {
        Ok(Table::BlacklistedIps
            .atomic_get(
                bincode::serialize(&address).expect("can serialize"),
                |result| bincode::deserialize(result),
            )
            .transpose()?)
    }

    pub fn get_all() -> Vec<BlacklistedIp> {
        Table::BlacklistedIps
            .range(..)
            .atomic_collect(|_, value| bincode::deserialize(value).expect("can deserialize"))
    }

    pub fn insert_with(&self, tx: &mut WritableTx) {
        Table::BlacklistedIps.put(
            tx,
            bincode::serialize(&self.address).expect("can serialize"),
            bincode::serialize(&self).expect("can serialize"),
        )
    }

    pub fn insert(&self) -> Result<(), crate::Error> {
        writable_tx(|tx| {
            self.insert_with(tx);
            Ok(())
        })
    }
}
