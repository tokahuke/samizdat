use serde_derive::{Deserialize, Serialize};
use std::net::IpAddr;

use samizdat_common::db::{Table as _, TxHandle, WritableTx};

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

    pub fn get<Tx: TxHandle>(
        tx: &Tx,
        address: IpAddr,
    ) -> Result<Option<BlacklistedIp>, crate::Error> {
        Ok(Table::BlacklistedIps
            .get(
                tx,
                bincode::serialize(&address).expect("can serialize"),
                |result| bincode::deserialize(result),
            )
            .transpose()?)
    }

    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Vec<BlacklistedIp> {
        Table::BlacklistedIps.range(..).collect(tx, |_, value| {
            bincode::deserialize(value).expect("can deserialize")
        })
    }

    pub fn insert(&self, tx: &mut WritableTx) {
        Table::BlacklistedIps.put(
            tx,
            bincode::serialize(&self.address).expect("can serialize"),
            bincode::serialize(&self).expect("can serialize"),
        )
    }
}
