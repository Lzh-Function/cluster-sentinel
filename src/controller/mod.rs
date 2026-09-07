//! The controller.
//!
//! In M1 the controller is *passive*: it discovers inventory, records what
//! integrations report, derives state, and persists all of it. It changes
//! nothing on the cluster — no `scontrol update`, no restarts, no remounts
//! (SPEC.md §113).
//!
//! The controller is not tied to any particular host. Which machine runs it is
//! deployment configuration (SPEC.md §42), and the schema does not assume there
//! is only one (SPEC.md §43).

pub mod agents;
pub mod api;
mod discovery;
pub mod registration;
mod server;

pub use agents::{AgentRegistry, AgentSession, RegistrationKind};
pub use discovery::{DiscoveryReport, ProviderReport};
pub use registration::{snapshot_from_registration, Registration};
pub use server::{serve, ServeOptions, ServerHandle};

use std::sync::Arc;

use crate::config::Config;
use crate::integrations::slurm::{observe, ScontrolClient};
use crate::inventory::slurm::SlurmInventoryProvider;
use crate::inventory::static_config::StaticConfigProvider;
use crate::inventory::InventoryProvider;
use crate::persistence::{SqliteStore, StoreError};
use crate::state::{ProbeMapping, StateComponent, StateEngine};

/// Name used for the scheduler entity when configuration does not name a
/// cluster. Deliberately generic: no deployment identifier belongs here.
pub const DEFAULT_SCHEDULER_NAME: &str = "slurm";

/// The controller's long-lived state.
pub struct Controller {
    config: Config,
    store: SqliteStore,
    providers: Vec<Arc<dyn InventoryProvider>>,
    engine: StateEngine,
}

impl Controller {
    /// Build a controller from configuration, wiring up the enabled providers.
    pub async fn new(config: Config, store: SqliteStore) -> Result<Self, StoreError> {
        store.ensure_environment(&config.environment).await?;

        let mut providers: Vec<Arc<dyn InventoryProvider>> = vec![Arc::new(StaticConfigProvider::new(config.clone()))];

        if config.discovery.slurm.enabled {
            providers.push(Arc::new(SlurmInventoryProvider::new(
                &config.environment,
                scheduler_name(&config),
                ScontrolClient::with_path(config.discovery.slurm.scontrol_path.clone()),
            )));
        }

        let mut engine = StateEngine::new();
        register_builtin_probes(&mut engine);

        // Resume from what is already known, so a restart does not re-learn
        // every host's state from scratch.
        for (_, state) in store.load_entity_states(&config.environment).await? {
            engine.seed(state);
        }

        Ok(Self {
            config,
            store,
            providers,
            engine,
        })
    }

    /// The configuration in force.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The database.
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// The state engine.
    pub fn engine(&self) -> &StateEngine {
        &self.engine
    }

    /// The state engine, mutably.
    pub fn engine_mut(&mut self) -> &mut StateEngine {
        &mut self.engine
    }

    /// Names of the enabled inventory providers.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.iter().map(|p| p.name()).collect()
    }

    /// The scheduler entity name in force.
    pub fn scheduler_name(&self) -> String {
        scheduler_name(&self.config)
    }
}

/// The scheduler entity's name: the configured cluster, else a generic default.
fn scheduler_name(config: &Config) -> String {
    config
        .entities
        .iter()
        .find(|e| e.entity_type == "scheduler")
        .map(|e| e.name.clone())
        .unwrap_or_else(|| DEFAULT_SCHEDULER_NAME.to_string())
}

/// Declare how the built-in probes feed into state.
///
/// This is the whole coupling between an integration and the state engine: a
/// probe id and the component it informs.
pub fn register_builtin_probes(engine: &mut StateEngine) {
    // A scheduler reporting DRAIN or DOWN is an authoritative statement, not a
    // flaky measurement, so it takes effect immediately (IMPLEMENTATION.md §70).
    engine.register(observe::PROBE_NODE, ProbeMapping::immediate(StateComponent::Scheduler));
    engine.register(
        observe::PROBE_CONTROLLER,
        ProbeMapping::immediate(StateComponent::Service),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EntityConfig, SlurmDiscoveryConfig};
    use crate::probes::ProbeId;

    fn config() -> Config {
        Config {
            config_version: 1,
            environment: "lab".into(),
            ..Config::default()
        }
    }

    async fn store() -> SqliteStore {
        SqliteStore::open_in_memory().await.expect("open")
    }

    #[tokio::test]
    async fn static_configuration_is_always_a_provider() {
        let controller = Controller::new(config(), store().await).await.expect("controller");
        assert_eq!(controller.provider_names(), ["static_config"]);
    }

    #[tokio::test]
    async fn slurm_is_only_a_provider_when_it_is_enabled() {
        let mut config = config();
        config.discovery.slurm = SlurmDiscoveryConfig {
            enabled: true,
            scontrol_path: None,
        };

        let controller = Controller::new(config, store().await).await.expect("controller");
        assert_eq!(controller.provider_names(), ["static_config", "slurm"]);
    }

    #[tokio::test]
    async fn the_scheduler_name_comes_from_configuration_not_from_the_binary() {
        let mut config = config();
        config.entities.push(EntityConfig {
            entity_type: "scheduler".into(),
            name: "research-cluster".into(),
            display_name: None,
            cluster: None,
            labels: Default::default(),
            capabilities: vec![],
            addresses: vec![],
        });

        let controller = Controller::new(config, store().await).await.expect("controller");
        assert_eq!(controller.scheduler_name(), "research-cluster");
    }

    #[tokio::test]
    async fn an_unnamed_scheduler_falls_back_to_a_generic_default() {
        let controller = Controller::new(config(), store().await).await.expect("controller");
        assert_eq!(controller.scheduler_name(), DEFAULT_SCHEDULER_NAME);
    }

    #[tokio::test]
    async fn creating_a_controller_registers_its_environment() {
        let store = store().await;
        Controller::new(config(), store.clone()).await.expect("controller");
        assert_eq!(
            store.environments().await.expect("environments"),
            vec!["lab".to_string()]
        );
    }

    #[test]
    fn the_builtin_probes_are_registered_with_the_state_engine() {
        let mut engine = StateEngine::new();
        register_builtin_probes(&mut engine);
        assert!(engine.knows(&ProbeId::new(observe::PROBE_NODE)));
        assert!(engine.knows(&ProbeId::new(observe::PROBE_CONTROLLER)));
        assert!(!engine.knows(&ProbeId::new("never.registered")));
    }
}
