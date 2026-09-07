//! Running the controller as a daemon.
//!
//! Two things run concurrently: the HTTP API agents talk to, and a periodic
//! discovery cycle. Either can fail without stopping the other — a Slurm
//! outage must not take the API down, and an API error must not stop
//! discovery.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio::sync::Mutex;

use crate::protocol::ClusterCredential;

use super::agents::AgentRegistry;
use super::api::{router, ApiState};
use super::Controller;

/// A running controller.
pub struct ServerHandle {
    /// The address actually bound, which may differ from the requested one when
    /// port 0 was asked for.
    pub local_addr: SocketAddr,
    shutdown: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
    discovery: Option<tokio::task::JoinHandle<()>>,
}

impl ServerHandle {
    /// Ask the server to stop and wait for it (SPEC.md §84).
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        if let Some(discovery) = self.discovery {
            discovery.abort();
            let _ = discovery.await;
        }
        let _ = self.server.await;
    }
}

/// Options for running a controller.
pub struct ServeOptions {
    /// Address to listen on.
    pub listen: String,
    /// The cluster credential agents must present.
    pub credential: ClusterCredential,
    /// How often agents should heartbeat.
    pub heartbeat_interval: std::time::Duration,
    /// How often to run inventory discovery; `None` disables the loop.
    pub discovery_interval: Option<std::time::Duration>,
}

/// Start the controller's API, and its discovery loop if one is configured.
pub async fn serve(controller: Controller, options: ServeOptions) -> anyhow::Result<ServerHandle> {
    let discovery_interval = options.discovery_interval;
    let controller = Arc::new(Mutex::new(controller));

    let state = ApiState {
        controller: Arc::clone(&controller),
        agents: Arc::new(Mutex::new(AgentRegistry::new())),
        credential: Arc::new(options.credential),
        heartbeat_interval: options.heartbeat_interval,
    };

    let listener = TcpListener::bind(&options.listen).await?;
    let local_addr = listener.local_addr()?;
    tracing::info!(%local_addr, "controller listening");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    let discovery = discovery_interval.map(|interval| {
        let controller = Arc::clone(&controller);
        tokio::spawn(async move { discovery_loop(controller, interval).await })
    });

    Ok(ServerHandle {
        local_addr,
        shutdown: shutdown_tx,
        server,
        discovery,
    })
}

/// Run discovery on a schedule.
async fn discovery_loop(controller: Arc<Mutex<Controller>>, interval: std::time::Duration) {
    // Jitter so a fleet of controllers, or a controller restarted in lockstep
    // with others, does not synchronise its load (SPEC.md §123).
    let jitter = jitter_for(interval);
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        tokio::time::sleep(jitter).await;

        let mut controller = controller.lock().await;
        match controller.discover_once().await {
            Ok(report) => {
                if !report.all_providers_ok() {
                    for failure in report.failures() {
                        tracing::warn!(
                            provider = %failure.provider,
                            error = failure.error.as_deref().unwrap_or_default(),
                            "discovery provider failed"
                        );
                    }
                }
                tracing::debug!(
                    entities = report.entities,
                    observations = report.observations,
                    transitions = report.transitions.len(),
                    "discovery cycle complete"
                );
            }
            // A failed cycle must never end the loop: the next one may succeed,
            // and a monitoring daemon that gives up during an outage is useless.
            Err(error) => tracing::error!(%error, "discovery cycle failed"),
        }
    }
}

/// A deterministic-per-process jitter of up to 10% of the interval.
fn jitter_for(interval: std::time::Duration) -> std::time::Duration {
    let span = interval.as_millis() as u64 / 10;
    if span == 0 {
        return std::time::Duration::ZERO;
    }
    // Derived from the process id rather than a random number generator: enough
    // to break lockstep between hosts, and reproducible within one process.
    std::time::Duration::from_millis(u64::from(std::process::id()) % span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::persistence::SqliteStore;
    use crate::protocol::{RegisterRequest, API_PREFIX};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    async fn start(discovery_interval: Option<std::time::Duration>) -> ServerHandle {
        let config = Config {
            config_version: 1,
            environment: "lab".into(),
            ..Config::default()
        };
        let store = SqliteStore::open_in_memory().await.expect("store");
        let controller = Controller::new(config, store).await.expect("controller");

        serve(
            controller,
            ServeOptions {
                // Port 0: the OS picks a free port, so tests never collide.
                listen: "127.0.0.1:0".into(),
                credential: ClusterCredential::new(TOKEN),
                heartbeat_interval: std::time::Duration::from_secs(5),
                discovery_interval,
            },
        )
        .await
        .expect("serve")
    }

    #[tokio::test]
    async fn the_server_binds_and_answers_health() {
        let handle = start(None).await;
        let url = format!("http://{}{API_PREFIX}/health", handle.local_addr);

        let body: serde_json::Value = reqwest::get(&url).await.expect("get").json().await.expect("json");
        assert_eq!(body["environment"], "lab");
        assert_eq!(body["protocol_version"], crate::PROTOCOL_VERSION);

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn an_agent_can_register_over_http() {
        let handle = start(None).await;
        let url = format!("http://{}{API_PREFIX}/agents/register", handle.local_addr);
        let request = RegisterRequest::new("lab", "node-a", ["host.metrics"].into_iter().collect());

        let response = reqwest::Client::new()
            .post(&url)
            .bearer_auth(TOKEN)
            .json(&request)
            .send()
            .await
            .expect("register");

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await.expect("json");
        assert!(body["session_id"].is_string());

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn registration_without_the_credential_is_refused_over_http() {
        let handle = start(None).await;
        let url = format!("http://{}{API_PREFIX}/agents/register", handle.local_addr);
        let request = RegisterRequest::new("lab", "node-a", Default::default());

        let response = reqwest::Client::new()
            .post(&url)
            .json(&request)
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_stops_the_listener() {
        let handle = start(None).await;
        let addr = handle.local_addr;
        handle.shutdown().await;

        // Give the OS a moment to release the socket, then confirm it is gone.
        for _ in 0..50 {
            if reqwest::get(format!("http://{addr}{API_PREFIX}/health")).await.is_err() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the server was still answering after shutdown");
    }

    #[tokio::test]
    async fn the_discovery_loop_runs_without_blocking_the_api() {
        let handle = start(Some(std::time::Duration::from_millis(50))).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let url = format!("http://{}{API_PREFIX}/health", handle.local_addr);
        assert!(reqwest::get(&url).await.expect("get").status().is_success());

        handle.shutdown().await;
    }

    #[test]
    fn jitter_is_bounded_by_a_tenth_of_the_interval() {
        for seconds in [1u64, 5, 60, 300] {
            let interval = std::time::Duration::from_secs(seconds);
            assert!(jitter_for(interval) < interval / 10 + std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn a_tiny_interval_produces_no_jitter_rather_than_dividing_by_zero() {
        assert_eq!(
            jitter_for(std::time::Duration::from_millis(5)),
            std::time::Duration::ZERO
        );
        assert_eq!(jitter_for(std::time::Duration::ZERO), std::time::Duration::ZERO);
    }
}
