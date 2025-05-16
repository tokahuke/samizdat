use std::time::Duration;

use samizdat_common::rpc::{Query, QueryResponse};
use serde_derive::{Deserialize, Serialize};
use serde_with::{serde_as, DurationMilliSecondsWithFrac};

use crate::db::Table;

use super::{Id, Indexable};

#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct QueryLog {
    /// The id of the connection that made the query.
    connection_id: Id,
    /// Id of the query log.
    query_log_id: Id,
    /// The query made by the client.
    query: Query,
    /// The response from the hub.
    response: Option<QueryResponse>,
    /// The duration of the query.
    #[serde_as(as = "Option<DurationMilliSecondsWithFrac>")]
    duration: Option<Duration>,
}

impl Indexable for QueryLog {
    const TABLE: Table = Table::QueryLog;

    fn id(&self) -> Id {
        self.query_log_id
    }
}

impl QueryLog {
    pub fn new(connection_id: Id, query: Query) -> Self {
        Self {
            connection_id,
            query_log_id: Id::generate(),
            query,
            response: None,
            duration: None,
        }
    }

    pub fn update_with_response(&mut self, response: QueryResponse, duration: Duration) {
        self.response = Some(response);
        self.duration = Some(duration);
    }
}
