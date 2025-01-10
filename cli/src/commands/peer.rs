//! Peer command implementations for the Samizdat CLI.

use tabled::Tabled;

use super::show_table;
use crate::api;

/// Lists all known peers and their current status.
pub async fn ls() -> Result<(), anyhow::Error> {
    let response = api::get_all_peers().await?;

    #[derive(Tabled)]
    struct Row {
        /// Network address of the peer
        addr: String,
        /// Current connection status
        status: String,
    }

    show_table(
        response
            .into_iter()
            .map(|peer| Row {
                addr: peer.addr,
                status: peer.status,
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}
