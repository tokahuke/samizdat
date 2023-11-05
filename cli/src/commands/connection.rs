use tabled::Tabled;

use crate::api::{self};

use super::show_table;

pub async fn ls() -> Result<(), anyhow::Error> {
    let response = api::get_all_connections().await?;

    #[derive(Tabled)]
    struct Row {
        name: String,
        status: String,
        direct_addr: String,
        reverse_addr: String,
    }

    show_table(
        response
            .into_iter()
            .map(|conn| Row {
                name: conn.name,
                status: conn.status,
                direct_addr: conn.direct_addr,
                reverse_addr: conn.reverse_addr,
            })
            .collect::<Vec<_>>(),
    );

    Ok(())
}
