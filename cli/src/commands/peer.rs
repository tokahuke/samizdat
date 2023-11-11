use tabled::Tabled;

use crate::api::{self};

use super::show_table;

pub async fn ls() -> Result<(), anyhow::Error> {
    let response = api::get_all_peers().await?;

    #[derive(Tabled)]
    struct Row {
        hub_name: String,
        addr: String,
        is_closed: bool,
    }

    show_table(
        response
            .into_iter()
            .map(|peer| Row {
                hub_name: peer.hub_name,
                addr: peer.addr,
                is_closed: peer.is_closed,
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}
