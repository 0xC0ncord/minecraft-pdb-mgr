use anyhow::{Context, Result, anyhow};
use futures::stream::StreamExt;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    Client,
    api::{Api, Patch, PatchParams},
};
use percentage::Percentage;
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook_tokio::Signals;
use std::sync::Arc;

const DEFAULT_UPDATE_INTERVAL_SECONDS: u64 = 10;
const DEFAULT_MIN_PLAYERS: u32 = 1;

#[tokio::main]
async fn main() {
    unsafe {
        std::env::set_var(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or("info".to_string()),
        );
    }
    env_logger::init();

    if let Err(e) = run().await {
        log::error!("Error: {e}");
        std::process::exit(1);
    }
}

async fn handle_signals(shutdown_notify: Arc<tokio::sync::Notify>) {
    let mut signals = Signals::new([SIGINT, SIGTERM]).unwrap();
    while let Some(signal) = signals.next().await {
        log::info!("Signal {signal} received, notifying shutdown.");
        shutdown_notify.notify_waiters();
    }
}

async fn get_server_player_info(host: &str, port: u16) -> Result<(u32, u32)> {
    match mc_query::status::status_with_timeout(host, port, std::time::Duration::from_secs(10))
        .await
    {
        Ok(s) => Ok((s.players.online, s.players.max)),
        Err(e) => Err(e.into()),
    }
}

async fn try_update_pdb(
    api: &Api<PodDisruptionBudget>,
    pdb_name: &str,
    min_players: &u32,
    min_players_pct: &f64,
    server_host: &str,
    server_port: &u16,
    last_has_players: &mut bool,
) -> Result<()> {
    let (players_online, players_max): (u32, u32) =
        match get_server_player_info(server_host, *server_port).await {
            Ok((online, max)) => (online, max),
            Err(e) => {
                return Err(anyhow!("Failed to get server player count: {e}"));
            }
        };
    let (players_needed, need_msg): (f64, String) = if *min_players_pct > 0.0 {
        let req: f64 = Percentage::from_decimal(*min_players_pct).apply_to(players_max.into());
        (
            req,
            format!("{:.0}% [{}]", *min_players_pct * 100.0, req as i32),
        )
    } else {
        (f64::from(*min_players), format!("{min_players}"))
    };
    let has_players = f64::from(players_online) >= players_needed;

    log::debug!(
        "Condition {}: {players_online}/{players_max} players (need {need_msg}).",
        if has_players { "met" } else { "unmet" }
    );

    if has_players == *last_has_players {
        log::debug!("Server player state unchanged - skipping this update.");
        return Ok(());
    }

    // Construct the patch.
    let patch = Patch::Merge(serde_json::json!({
        "spec": {
            "maxUnavailable": u32::from(!has_players)
        }
    }));
    // Patch it.
    match api.patch(pdb_name, &PatchParams::default(), &patch).await {
        Ok(_) => {
            log::debug!("PodDisruptionBudget {pdb_name} patched successfully.");
            *last_has_players = has_players;
            Ok(())
        }
        Err(e) => Err(anyhow!(
            "Failed to patch PodDisruptionBudget {pdb_name}: {e}"
        )),
    }
}

async fn run() -> Result<()> {
    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    tokio::spawn(handle_signals(shutdown_notify.clone()));

    // Grab required values from env vars.
    let update_interval: u64 = std::env::var("UPDATE_INTERVAL")
        .unwrap_or(DEFAULT_UPDATE_INTERVAL_SECONDS.to_string())
        .parse()
        .context("UPDATE_INTERVAL conversion to u64 failed!")?;
    let pod_namespace: String = std::env::var("POD_NAMESPACE")
        .context("Could not determine pod namespace from POD_NAMESPACE!")?;
    let pdb_name: String = std::env::var("PDB_NAME").context("No PDB_NAME specified!")?;
    let min_players: u32 = match std::env::var("MIN_PLAYERS") {
        Ok(s) => s.parse().context("MIN_PLAYERS conversion to u32 failed!")?,
        Err(_) => DEFAULT_MIN_PLAYERS,
    };
    let min_players_pct: f64 = match std::env::var("MIN_PLAYERS_PERCENT") {
        Ok(s) => s
            .parse()
            .context("MIN_PLAYERS_PERCENT conversion to f64 failed!")?,
        Err(_) => 0.0,
    };
    let server_host: String = std::env::var("SERVER_HOST").context("No SERVER_HOST specified!")?;
    let server_port: u16 = std::env::var("SERVER_PORT")
        .context("No SERVER_PORT specified!")?
        .parse()
        .context("SERVER_PORT conversion to u16 failed!")?;

    if std::env::var("RUST_LOG")?.to_lowercase() == "debug" {
        if min_players_pct > 0.0 {
            log::debug!(
                "Will watch for minimum {:.0}% of players.",
                min_players_pct * 100.0
            );
        } else {
            log::debug!("Will watch for minimum {min_players} players.");
        }
    }

    // Set up required Kube client.
    let client = Client::try_default().await?;
    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &pod_namespace);
    // Check the initial state of the PDB.
    let pdb = api.get(&pdb_name).await;

    // Save its current state if possible.
    let mut last_has_players: bool = pdb.map_or_else(
        |e| {
            log::warn!("{e}");
            false
        },
        |v| {
            matches!(
                v.spec.as_ref().and_then(|s| s.max_unavailable.as_ref()),
                Some(IntOrString::Int(0))
            )
        },
    );

    // Wrap the update method in an error printer.
    let mut do_update = async || {
        if let Err(e) = try_update_pdb(
            &api,
            &pdb_name,
            &min_players,
            &min_players_pct,
            &server_host,
            &server_port,
            &mut last_has_players,
        )
        .await
        {
            log::warn!("{e}");
        }
    };
    // Try initial update.
    do_update().await;

    // Now start running.
    loop {
        tokio::select! {
            // Shut down if we received a signal.
            _ = shutdown_notify.notified() => {
                log::info!("Shutting down.");
                break;
            },
            // The main loop.
            _ = tokio::time::sleep(std::time::Duration::from_secs(update_interval)) => {
                do_update().await;
            }
        }
    }

    Ok(())
}
