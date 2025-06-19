use anyhow::{Context, Result, anyhow};
use futures::stream::StreamExt;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::{
    Client,
    api::{Api, Patch, PatchParams},
};
use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook_tokio::Signals;
use std::sync::Arc;

const DEFAULT_UPDATE_INTERVAL_SECONDS: u64 = 10;

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

async fn get_server_player_count(host: &str, port: u16) -> Result<u32> {
    match mc_query::status::status_with_timeout(host, port, std::time::Duration::from_secs(10))
        .await
    {
        Ok(s) => Ok(s.players.online),
        Err(e) => Err(e.into()),
    }
}

async fn try_update_pdb(
    api: &Api<PodDisruptionBudget>,
    pdb_name: &str,
    server_host: &str,
    server_port: &u16,
    last_has_players: &mut bool,
) -> Result<()> {
    let has_players = match get_server_player_count(server_host, *server_port).await {
        Ok(i) => i > 0,
        Err(e) => {
            return Err(anyhow!("Failed to get server player count: {e}"));
        }
    };
    if has_players == *last_has_players {
        log::debug!("Server player state unchanged - skipping this update.");
        return Ok(());
    } else if has_players {
        log::debug!("Server player state changed - one or more players connected.");
    } else {
        log::debug!("Server player state changed - no players connected.");
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
    let server_host: String = std::env::var("SERVER_HOST").context("No SERVER_HOST specified!")?;
    let server_port: u16 = std::env::var("SERVER_PORT")
        .context("No SERVER_PORT specified!")?
        .parse()
        .context("SERVER_PORT conversion to u16 failed!")?;

    // Set up required Kube client.
    let client = Client::try_default().await?;
    let api: Api<PodDisruptionBudget> = Api::namespaced(client.clone(), &pod_namespace);
    // Check the initial state of the PDB.
    let pdb = api.get(&pdb_name).await;

    // Save its current state if possible.
    let mut last_has_players: bool = match pdb {
        Ok(v) => match v.spec.unwrap().max_unavailable {
            Some(IntOrString::Int(i)) => i == 0,
            Some(IntOrString::String(_)) => false,
            None => false,
        },
        Err(e) => {
            log::warn!("{e}");
            false
        }
    };

    // Wrap the update method in an error printer.
    let mut do_update = async || {
        if let Err(e) = try_update_pdb(
            &api,
            &pdb_name,
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
