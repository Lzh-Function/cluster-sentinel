//! The Sentinel agent.
//!
//! One agent binary runs on every host (SPEC.md §44). Its responsibilities in
//! this milestone are: work out what this machine can do, tell a controller,
//! stay in touch, and never lose an observation because the controller was
//! unreachable.
//!
//! The agent is built around one operating assumption: **the controller will be
//! unavailable sometimes, and that is not an error.** Registration retries,
//! heartbeats tolerate gaps, and observations go to a local spool that is
//! replayed on reconnection.

pub mod client;
pub mod discovery;
pub mod spool;
pub mod system;

pub use client::{ClientError, ControllerClient};
pub use spool::{Spool, SpoolLimits};
pub use system::{LinuxInspector, SystemInspector};

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::capability::CapabilitySet;
use crate::config::Config;
use crate::observation::Observation;
use crate::protocol::{HeartbeatRequest, ObservationBatch, RegisterRequest};
use crate::time::now;
use crate::PROTOCOL_VERSION;

/// The largest batch the agent sends at once.
///
/// Bounded so that a long outage does not turn into one enormous request that
/// times out, fails, and is retried identically forever.
pub const MAX_BATCH: u32 = 500;

/// How much an agent knows about its own situation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentStatus {
    /// Whether the agent is currently registered.
    pub registered: bool,
    /// The agent's identity, once registered.
    pub agent_id: Option<Uuid>,
    /// The current session, once registered.
    pub session_id: Option<Uuid>,
    /// Observations waiting to be delivered.
    pub spooled: u64,
    /// Measured clock difference against the controller, in milliseconds.
    pub clock_skew_ms: Option<i64>,
    /// Why the last attempt to reach the controller failed.
    pub last_error: Option<String>,
}

/// A Sentinel agent.
pub struct Agent {
    environment: String,
    hostname: String,
    inspector: Arc<dyn SystemInspector>,
    client: ControllerClient,
    spool: Spool,
    capabilities: CapabilitySet,
    roles: Vec<String>,
    status: AgentStatus,
}

impl Agent {
    /// Build an agent from configuration and a view of the local system.
    ///
    /// Fails only if the host name cannot be determined: an agent that does not
    /// know which machine it is on cannot report anything meaningful, and
    /// guessing would attach observations to the wrong entity.
    pub fn new(
        config: &Config,
        inspector: Arc<dyn SystemInspector>,
        client: ControllerClient,
        spool: Spool,
    ) -> anyhow::Result<Self> {
        let hostname = inspector
            .hostname()
            .ok_or_else(|| anyhow::anyhow!("cannot determine this host's name; refusing to guess"))?;

        let resolution = discovery::resolve(inspector.as_ref(), &config.capabilities, &config.agent.roles);

        Ok(Self {
            environment: config.environment.clone(),
            hostname,
            inspector,
            client,
            spool,
            capabilities: resolution.enabled,
            roles: config.agent.roles.clone(),
            status: AgentStatus::default(),
        })
    }

    /// The host name this agent reports as.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// The capabilities in force.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Current status.
    pub fn status(&self) -> &AgentStatus {
        &self.status
    }

    /// The local spool.
    pub fn spool(&self) -> &Spool {
        &self.spool
    }

    /// Build the registration this agent would send.
    pub fn registration(&self) -> RegisterRequest {
        let gpus = self
            .capabilities
            .has(crate::capability::well_known::GPU_NVIDIA)
            .then_some(0);

        RegisterRequest {
            protocol_version: PROTOCOL_VERSION,
            agent_version: crate::VERSION.to_string(),
            environment: self.environment.clone(),
            hostname: self.hostname.clone(),
            fqdn: self.inspector.fqdn(),
            boot_id: self.inspector.boot_id(),
            addresses: self.inspector.addresses(),
            capabilities: self.capabilities.clone(),
            hardware: serde_json::to_value(discovery::hardware(self.inspector.as_ref(), gpus))
                .unwrap_or(serde_json::Value::Null),
            roles: self.roles.clone(),
        }
    }

    /// Register with the controller.
    pub async fn register(&mut self) -> Result<(), ClientError> {
        match self.client.register(&self.registration()).await {
            Ok(response) => {
                self.status.registered = true;
                self.status.agent_id = Some(response.agent_id);
                self.status.session_id = Some(response.session_id);
                self.status.last_error = None;
                tracing::info!(
                    agent_id = %response.agent_id,
                    entity_id = %response.entity_id,
                    capabilities = self.capabilities.len(),
                    "registered with controller"
                );
                Ok(())
            }
            Err(error) => {
                self.status.registered = false;
                self.status.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// Send a heartbeat, re-registering if the controller asks.
    pub async fn heartbeat(&mut self) -> Result<(), ClientError> {
        let (Some(agent_id), Some(session_id)) = (self.status.agent_id, self.status.session_id) else {
            return self.register().await;
        };

        let request = HeartbeatRequest {
            protocol_version: PROTOCOL_VERSION,
            agent_id,
            session_id,
            boot_id: self.inspector.boot_id(),
            agent_time: now(),
            spooled_observations: self.spool.len().await.unwrap_or(0),
        };

        match self.client.heartbeat(&request).await {
            Ok(response) => {
                self.status.clock_skew_ms = Some(response.clock_skew_ms);
                self.status.last_error = None;
                if response.reregister {
                    tracing::info!("controller asked us to register again");
                    return self.register().await;
                }
                Ok(())
            }
            Err(error) => {
                self.status.last_error = Some(error.to_string());
                if !error.is_transient() {
                    self.status.registered = false;
                }
                Err(error)
            }
        }
    }

    /// Record observations, sending them if possible and spooling them if not.
    ///
    /// Observations are written to the spool *first*, then removed once the
    /// controller confirms them. Sending first and spooling on failure would
    /// lose everything to a crash between the two.
    pub async fn record(&mut self, observations: &[Observation]) -> anyhow::Result<()> {
        self.spool.push(observations).await?;
        self.flush().await?;
        Ok(())
    }

    /// Try to deliver everything in the spool.
    ///
    /// Returns how many observations the controller accepted. A failure here is
    /// not an error for the caller: the observations remain spooled, and the
    /// next attempt will carry them.
    pub async fn flush(&mut self) -> anyhow::Result<usize> {
        let (Some(agent_id), Some(session_id)) = (self.status.agent_id, self.status.session_id) else {
            // Not registered yet. The spool keeps growing, bounded by its
            // limits, until there is somewhere to send it.
            self.status.spooled = self.spool.len().await.unwrap_or(0);
            return Ok(0);
        };

        let mut delivered = 0;
        loop {
            let batch = self.spool.peek(MAX_BATCH).await?;
            if batch.is_empty() {
                break;
            }

            let request = ObservationBatch {
                protocol_version: PROTOCOL_VERSION,
                agent_id,
                session_id,
                observations: batch.clone(),
            };

            match self.client.send_observations(&request).await {
                Ok(response) => {
                    // Acknowledge duplicates too: the controller already has
                    // them, so holding on would replay them forever.
                    let rejected: std::collections::HashSet<&str> =
                        response.rejected.iter().map(|r| r.id.as_str()).collect();
                    let confirmed: Vec<_> = batch
                        .iter()
                        .filter(|o| !rejected.contains(o.id.to_string().as_str()))
                        .map(|o| o.id)
                        .collect();

                    self.spool.acknowledge(&confirmed).await?;
                    delivered += response.accepted;
                    self.status.last_error = None;

                    if !response.rejected.is_empty() {
                        // Rejected observations stay spooled: the entity may
                        // appear later, and dropping them would hide a real
                        // configuration problem.
                        tracing::warn!(count = response.rejected.len(), "controller rejected observations");
                        break;
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, spooled = batch.len(), "cannot deliver observations; they stay spooled");
                    self.status.last_error = Some(error.to_string());
                    break;
                }
            }
        }

        self.status.spooled = self.spool.len().await.unwrap_or(0);
        Ok(delivered)
    }

    /// Run until cancelled: register, then heartbeat and flush on a schedule.
    pub async fn run(&mut self, heartbeat_interval: Duration, shutdown: tokio::sync::oneshot::Receiver<()>) {
        // A failed first registration is not fatal: the controller may simply
        // not be up yet, and the agent must keep trying while spooling.
        if let Err(error) = self.register().await {
            tracing::warn!(%error, "initial registration failed; will retry");
        }

        let mut ticker = tokio::time::interval(heartbeat_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(error) = self.heartbeat().await {
                        tracing::debug!(%error, "heartbeat failed");
                    }
                    if let Err(error) = self.flush().await {
                        tracing::warn!(%error, "flushing the spool failed");
                    }
                }
                _ = &mut shutdown => {
                    tracing::info!("agent shutting down");
                    // One last attempt to hand over what we have.
                    let _ = self.flush().await;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::system::FakeInspector;
    use crate::config::AgentConfig;
    use crate::entity::{EntityKey, EntityType};
    use crate::observation::ProbeStatus;
    use crate::probes::ProbeId;
    use crate::protocol::ClusterCredential;

    fn config(roles: Vec<String>) -> Config {
        Config {
            config_version: 1,
            environment: "lab".into(),
            agent: AgentConfig {
                controller_address: None,
                spool_path: None,
                roles,
            },
            ..Config::default()
        }
    }

    async fn agent_with(inspector: FakeInspector, address: &str) -> Agent {
        let credential = ClusterCredential::new("0123456789abcdef0123456789abcdef");
        Agent::new(
            &config(vec![]),
            Arc::new(inspector),
            ControllerClient::new(address, &credential, Duration::from_millis(200)).expect("client"),
            Spool::open_in_memory(SpoolLimits::default()).await.expect("spool"),
        )
        .expect("agent")
    }

    fn observation() -> Observation {
        Observation::new(
            ProbeId::new("host.metrics"),
            EntityKey::new("lab", EntityType::Host, "test-host").entity_id(),
            ProbeStatus::Ok,
        )
    }

    #[tokio::test]
    async fn an_agent_reports_the_capabilities_discovery_found() {
        let inspector = FakeInspector::bare()
            .with_program("slurmd")
            .with_mount("fs:/export", "/home", "nfs4");
        let agent = agent_with(inspector, "127.0.0.1:1").await;

        assert!(agent.capabilities().has("slurm.compute"));
        assert!(agent.capabilities().has("storage.nfs.client"));
        assert!(!agent.capabilities().has("gpu.nvidia"));
    }

    #[tokio::test]
    async fn an_agent_without_a_host_name_refuses_to_start() {
        // Guessing would attach this machine's observations to another entity.
        let inspector = FakeInspector {
            hostname: None,
            ..FakeInspector::bare()
        };
        let credential = ClusterCredential::new("0123456789abcdef0123456789abcdef");
        let result = Agent::new(
            &config(vec![]),
            Arc::new(inspector),
            ControllerClient::new("127.0.0.1:1", &credential, Duration::from_millis(100)).expect("client"),
            Spool::open_in_memory(SpoolLimits::default()).await.expect("spool"),
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn the_registration_carries_identity_addresses_and_hardware() {
        let inspector = FakeInspector::bare().with_hostname("node-a").with_boot_id("boot-7");
        let request = agent_with(inspector, "127.0.0.1:1").await.registration();

        assert_eq!(request.hostname, "node-a");
        assert_eq!(request.boot_id.as_deref(), Some("boot-7"));
        assert_eq!(request.addresses, ["192.0.2.1"]);
        assert_eq!(request.hardware["cpus"], 8);
        assert_eq!(request.protocol_version, PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn a_role_is_reported_as_a_role_and_not_as_a_capability() {
        let credential = ClusterCredential::new("0123456789abcdef0123456789abcdef");
        let agent = Agent::new(
            &config(vec!["fileserver".into()]),
            Arc::new(FakeInspector::bare()),
            ControllerClient::new("127.0.0.1:1", &credential, Duration::from_millis(100)).expect("client"),
            Spool::open_in_memory(SpoolLimits::default()).await.expect("spool"),
        )
        .expect("agent");

        let request = agent.registration();
        assert_eq!(request.roles, ["fileserver"]);
        assert!(
            !request.capabilities.has("storage.nfs.server"),
            "this host exports nothing; the label must not conjure the capability"
        );
    }

    #[tokio::test]
    async fn registration_against_an_unreachable_controller_fails_without_panicking() {
        let mut agent = agent_with(FakeInspector::bare(), "127.0.0.1:1").await;
        assert!(agent.register().await.is_err());
        assert!(!agent.status().registered);
        assert!(agent.status().last_error.is_some());
    }

    #[tokio::test]
    async fn observations_are_spooled_when_the_controller_is_unreachable() {
        // SPEC.md §105: a controller outage is exactly when the evidence
        // matters most.
        let mut agent = agent_with(FakeInspector::bare(), "127.0.0.1:1").await;
        agent.record(&[observation(), observation()]).await.expect("record");

        assert_eq!(agent.spool().len().await.expect("len"), 2);
        assert_eq!(agent.status().spooled, 2);
    }

    #[tokio::test]
    async fn flushing_while_unregistered_keeps_everything_spooled() {
        let mut agent = agent_with(FakeInspector::bare(), "127.0.0.1:1").await;
        agent.record(&[observation()]).await.expect("record");
        assert_eq!(agent.flush().await.expect("flush"), 0);
        assert_eq!(agent.spool().len().await.expect("len"), 1, "nothing is lost");
    }

    #[tokio::test]
    async fn the_spool_bounds_growth_during_a_long_outage() {
        let credential = ClusterCredential::new("0123456789abcdef0123456789abcdef");
        let mut agent = Agent::new(
            &config(vec![]),
            Arc::new(FakeInspector::bare()),
            ControllerClient::new("127.0.0.1:1", &credential, Duration::from_millis(100)).expect("client"),
            Spool::open_in_memory(SpoolLimits {
                max_rows: 10,
                ..Default::default()
            })
            .await
            .expect("spool"),
        )
        .expect("agent");

        for _ in 0..50 {
            agent.record(&[observation()]).await.expect("record");
        }
        assert_eq!(agent.spool().len().await.expect("len"), 10, "the disk must not fill up");
    }
}
