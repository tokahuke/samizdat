use serde_derive::{Deserialize, Serialize};

use crate::db::Table;
use crate::models::Id;
use crate::models::Indexable;
use crate::rpc::node_sampler::StatisticsSnapshot;

#[derive(Debug, Serialize, Deserialize)]
pub struct StatisticsLog {
    id: Id,
    statistics: StatisticsSnapshot,
}

impl Indexable for StatisticsLog {
    const TABLE: Table = Table::StatisticsLog;

    fn id(&self) -> Id {
        self.id
    }
}

impl StatisticsLog {
    pub fn new(id: Id, statistics: StatisticsSnapshot) -> Self {
        Self { id, statistics }
    }

    pub fn statistics(&self) -> &StatisticsSnapshot {
        &self.statistics
    }
}
