//! `samizdat doctor` -- one-shot health snapshot of the connected node.
//!
//! Composes the existing `/_hubs`, `/_connections`, `/_peers`,
//! `/_subscriptions`, and `/_editions` endpoints into a single report.
//! Designed for triage rather than parsing: a human looks at it and
//! sees, in order, whether the node is alive, what hubs it knows
//! about, which links are actually up, what it has subscribed to, and
//! the most recent editions it has heard about. Output is plain text
//! so it pipes cleanly into a paste buffer or a CI artifact.

use crate::api;

pub async fn run() -> Result<(), anyhow::Error> {
    // The top-level `main` already called `validate_node_is_up`, so by
    // the time we get here the node has answered at least one HTTP
    // request. Anything we hit below that fails is a real signal.

    println!("== node ==");
    println!("  data dir:      {}", crate::cli::cli().data.display());
    match crate::access_token::port() {
        Ok(p) => println!("  http port:     {p}"),
        Err(e) => println!("  http port:     unknown ({e})"),
    }

    println!();
    println!("== hubs configured ==");
    match api::get_all_hubs().await {
        Ok(hubs) if hubs.is_empty() => println!("  (none)"),
        Ok(hubs) => {
            for h in &hubs {
                println!("  - {} (resolution_mode = {})", h.address, h.resolution_mode);
            }
        }
        Err(e) => println!("  ERROR: {e}"),
    }

    println!();
    println!("== live hub connections ==");
    match api::get_all_connections().await {
        Ok(conns) if conns.is_empty() => println!(
            "  (none) -- the node is not currently linked to any hub. Outgoing \
             announcements will go nowhere; incoming announcements will not \
             arrive. Most likely cause: hub address unreachable, or QUIC \
             handshake failing."
        ),
        Ok(conns) => {
            for c in &conns {
                println!("  - {} ({})", c.name, c.status);
            }
        }
        Err(e) => println!("  ERROR: {e}"),
    }

    println!();
    println!("== peers ==");
    match api::get_all_peers().await {
        Ok(peers) if peers.is_empty() => println!("  (none)"),
        Ok(peers) => {
            for p in &peers {
                println!("  - {} ({})", p.addr, p.status);
            }
        }
        Err(e) => println!("  ERROR: {e}"),
    }

    println!();
    println!("== subscriptions ==");
    match api::get_all_subscriptions().await {
        Ok(subs) if subs.is_empty() => println!("  (none)"),
        Ok(subs) => {
            for s in &subs {
                println!("  - {} ({:?})", s.public_key, s.kind);
            }
        }
        Err(e) => println!("  ERROR: {e}"),
    }

    println!();
    println!("== recent editions ==");
    match api::get_all_editions().await {
        Ok(editions) if editions.is_empty() => println!(
            "  (none) -- this node has not seen any edition. If it should \
             be receiving them via a subscription, check hub connections \
             above."
        ),
        Ok(editions) => {
            // Cap the output so a node that has been running for months
            // does not spam the terminal.
            for e in editions.iter().take(20) {
                println!("  - series={} timestamp={}", e.public_key, e.signed.timestamp);
            }
            if editions.len() > 20 {
                println!("  ... and {} more", editions.len() - 20);
            }
        }
        Err(e) => println!("  ERROR: {e}"),
    }

    Ok(())
}
