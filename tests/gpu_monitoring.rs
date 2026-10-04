//! GPU monitoring follows the agent's current command availability, while
//! historical observations remain available for inspection.

use sentinel::capability::CapabilitySet;
use sentinel::config::Config;
use sentinel::controller::Controller;
use sentinel::entity::{DiscoverySource, EntityKey, EntityType, ManagedEntity};
use sentinel::observation::{Observation, ProbeStatus};
use sentinel::persistence::SqliteStore;
use sentinel::probes::ProbeId;
use sentinel::protocol::RegisterRequest;
use sentinel::state::{ComponentState, EntityState, Health, StateComponent};

fn config() -> Config {
    Config {
        config_version: 1,
        environment: "lab".into(),
        ..Config::default()
    }
}

fn registration(gpu: bool) -> RegisterRequest {
    let mut capabilities = CapabilitySet::from_iter(["host.metrics"]);
    if gpu {
        capabilities.insert("gpu.nvidia");
    }
    RegisterRequest::new("lab", "node-a", capabilities)
}

fn observation(probe: &str, status: ProbeStatus) -> Observation {
    Observation::new(
        ProbeId::new(probe),
        EntityKey::new("lab", EntityType::Host, "node-a").entity_id(),
        status,
    )
}

async fn healthy_host(controller: &mut Controller) {
    controller
        .ingest_observations(&[
            observation("host.metrics", ProbeStatus::Ok),
            observation("host.metrics", ProbeStatus::Ok),
        ])
        .await
        .expect("host metrics");
}

#[tokio::test]
async fn removing_the_command_excludes_old_gpu_state_and_delayed_uploads_across_restarts() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    let mut controller = Controller::new(config(), store.clone()).await.expect("controller");
    let host = controller
        .register_agent(&registration(true))
        .await
        .expect("register")
        .entity_id;
    healthy_host(&mut controller).await;
    let mut gpu = observation("gpu.nvidia", ProbeStatus::Ok);
    gpu.started_at -= chrono::Duration::days(10);
    gpu.finished_at -= chrono::Duration::days(10);
    let mut second_gpu = gpu.clone();
    second_gpu.id = sentinel::observation::ObservationId::new();
    controller
        .ingest_observations(&[gpu.clone(), second_gpu])
        .await
        .expect("old GPU");
    controller.diagnose_and_classify().await.expect("expire");
    assert_eq!(
        controller
            .engine()
            .state(host)
            .unwrap()
            .component(StateComponent::Accelerator),
        Health::Unknown
    );

    controller
        .register_agent(&registration(false))
        .await
        .expect("command removed");
    let delayed = observation("gpu.nvidia", ProbeStatus::Failed);
    controller
        .ingest_observations(std::slice::from_ref(&delayed))
        .await
        .expect("delayed upload");
    controller.diagnose_and_classify().await.expect("diagnose");
    assert_eq!(controller.engine().state(host).unwrap().overall, Health::Healthy);
    assert_eq!(
        controller
            .engine()
            .state(host)
            .unwrap()
            .component(StateComponent::Accelerator),
        Health::NotApplicable
    );
    assert_eq!(
        store
            .recent_observations_for_probe(host, "gpu.nvidia", 10)
            .await
            .unwrap()
            .len(),
        3
    );
    assert_eq!(store.observation_by_id(gpu.id).await.unwrap(), Some(gpu));
    assert!(!controller.engine().confirms_health(&delayed));

    let mut restarted = Controller::new(config(), store.clone()).await.expect("restart");
    restarted.diagnose_and_classify().await.expect("diagnose after restart");
    assert_eq!(restarted.engine().state(host).unwrap().overall, Health::Healthy);
    assert_eq!(
        store.load_entity_states("lab").await.unwrap()[&host].overall,
        Health::Healthy
    );
}

#[tokio::test]
async fn restoring_the_command_requires_new_gpu_measurements() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    let mut controller = Controller::new(config(), store).await.expect("controller");
    let host = controller
        .register_agent(&registration(true))
        .await
        .expect("register")
        .entity_id;
    healthy_host(&mut controller).await;
    controller
        .ingest_observations(&[
            observation("gpu.nvidia", ProbeStatus::Ok),
            observation("gpu.nvidia", ProbeStatus::Ok),
        ])
        .await
        .expect("GPU metrics");
    controller.register_agent(&registration(false)).await.expect("remove");
    controller.register_agent(&registration(true)).await.expect("restore");
    assert_eq!(
        controller
            .engine()
            .state(host)
            .unwrap()
            .component(StateComponent::Accelerator),
        Health::Unknown
    );
    controller
        .ingest_observations(&[observation("gpu.nvidia", ProbeStatus::Ok)])
        .await
        .expect("first fresh GPU");
    assert_eq!(
        controller
            .engine()
            .state(host)
            .unwrap()
            .component(StateComponent::Accelerator),
        Health::Unknown
    );
    controller
        .ingest_observations(&[observation("gpu.nvidia", ProbeStatus::Ok)])
        .await
        .expect("second fresh GPU");
    assert_eq!(controller.engine().state(host).unwrap().overall, Health::Healthy);
}

#[tokio::test]
async fn an_existing_command_with_a_driver_error_still_reports_unknown() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    let mut controller = Controller::new(config(), store).await.expect("controller");
    let host = controller
        .register_agent(&registration(true))
        .await
        .expect("register")
        .entity_id;
    healthy_host(&mut controller).await;
    controller
        .ingest_observations(&[observation("gpu.nvidia", ProbeStatus::Unsupported)])
        .await
        .expect("driver error");
    assert_eq!(
        controller
            .engine()
            .state(host)
            .unwrap()
            .component(StateComponent::Accelerator),
        Health::Unknown
    );
}

#[tokio::test]
async fn a_static_gpu_hint_cannot_override_the_agents_missing_command() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    let mut controller = Controller::new(config(), store.clone()).await.expect("controller");
    let mut snapshot = sentinel::inventory::InventorySnapshot::new(DiscoverySource::StaticConfig);
    let mut host = ManagedEntity::new("lab", EntityType::Host, "node-a");
    host.capabilities = CapabilitySet::from_iter(["gpu.nvidia"]);
    snapshot.add_entity(host.clone());
    controller.ingest_snapshot(&snapshot).await.expect("static GPU hint");
    controller
        .register_agent(&registration(true))
        .await
        .expect("GPU command present");
    healthy_host(&mut controller).await;
    controller
        .ingest_observations(&[observation("gpu.nvidia", ProbeStatus::Unsupported)])
        .await
        .expect("GPU error");
    controller
        .register_agent(&registration(false))
        .await
        .expect("GPU command removed");
    controller
        .ingest_snapshot(&snapshot)
        .await
        .expect("rediscover static GPU hint");
    assert!(store
        .load_inventory("lab")
        .await
        .unwrap()
        .get(host.id)
        .unwrap()
        .capabilities
        .has("gpu.nvidia"));
    assert_eq!(controller.engine().state(host.id).unwrap().overall, Health::Healthy);
    let restarted = Controller::new(config(), store).await.expect("restart");
    assert!(!restarted.engine().probe_enabled(host.id, &ProbeId::new("gpu.nvidia")));
}

#[tokio::test]
async fn upgrading_an_old_registration_clears_gpu_state_even_after_its_history_was_pruned() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    store.ensure_environment("lab").await.unwrap();
    let mut host = ManagedEntity::new("lab", EntityType::Host, "node-a");
    host.discovery_sources.push(DiscoverySource::AgentRegistration);
    host.capabilities = CapabilitySet::from_iter(["host.metrics"]);
    host.metadata = serde_json::json!({"agent": {"version": "1.0.6"}});
    store.save_entity(&host).await.unwrap();
    let mut state = EntityState::unknown(host.id);
    state.set_component(StateComponent::Host, ComponentState::new(Health::Healthy));
    state.set_component(StateComponent::Accelerator, ComponentState::new(Health::Unknown));
    store.save_entity_state(&state).await.unwrap();

    let controller = Controller::new(config(), store.clone()).await.expect("upgrade");
    assert_eq!(controller.engine().state(host.id).unwrap().overall, Health::Healthy);
    assert_eq!(
        store.load_entity_states("lab").await.unwrap()[&host.id].overall,
        Health::Healthy
    );
}
