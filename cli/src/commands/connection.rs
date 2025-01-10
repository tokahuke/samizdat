//! Connection command implementations for the Samizdat CLI.
//!

use tabled::Tabled;

use super::show_table;
use crate::api;

/// Lists all active network connections.
pub async fn ls() -> Result<(), anyhow::Error> {
    let response = api::get_all_connections().await?;

    #[derive(Tabled)]
    struct Row {
        /// Name of the connection
        name: String,
        /// Current connection status
        status: String,
        /// Network address
        addr: String,
    }

    show_table(
        response
            .into_iter()
            .map(|conn| Row {
                name: conn.name,
                status: conn.status,
                addr: conn.addr,
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}
