use rocksdb::IteratorMode;
use rocksdb::WriteBatch;
use serde_derive::{Deserialize, Serialize};

use samizdat_common::address::{AddrResolutionMode, AddrToResolve};
use serde_with::serde_as;
use serde_with::DisplayFromStr;

use crate::db;
use crate::db::Table;

use super::Droppable;

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hub {
    #[serde_as(as = "DisplayFromStr")]
    pub address: AddrToResolve,
    pub resolution_mode: AddrResolutionMode,
}

impl Droppable for Hub {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        let address = self.address.to_string();
        batch.delete_cf(Table::Hubs.get(), &address);
        tokio::spawn(async move { crate::hubs().remove(&address).await });
        Ok(())
    }
}

impl Hub {
    /// Inserts the current identity in the database using the supplied [`WriteBatch`].
    pub fn insert(&self, batch: &mut WriteBatch) {
        let hub = self.clone();
        tokio::spawn(async move { crate::hubs().insert(hub).await });
        batch.put_cf(
            Table::Hubs.get(),
            self.address.to_string(),
            bincode::serialize(&self).expect("can serialize"),
        );
    }

    /// Lists all hubs currently in the database.
    pub fn get_all() -> Result<Vec<Hub>, crate::Error> {
        db().iterator_cf(Table::Hubs.get(), IteratorMode::Start)
            .map(|item| {
                let (_, value) = item?;
                Ok(bincode::deserialize(&value)?)
            })
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    pub fn get(address: AddrToResolve) -> Result<Option<Hub>, crate::Error> {
        let maybe_value = db().get_cf(Table::Hubs.get(), address.to_string())?;

        Ok(maybe_value
            .map(|value| bincode::deserialize(&value))
            .transpose()?)
    }
}
