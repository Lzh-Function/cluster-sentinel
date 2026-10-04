//! Updates reset derived state, not the observations used to explain history.

use sentinel::capability::CapabilitySet;
use sentinel::config::Config;
use sentinel::controller::Controller;
use sentinel::entity::{EntityKey, EntityType, ManagedEntity};
use sentinel::incident::{Incident, IncidentStatus, Severity};
use sentinel::observation::{Observation, ObservationId, ProbeStatus};
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

fn host() -> sentinel::entity::EntityId {
    EntityKey::new("lab", EntityType::Host, "node-a").entity_id()
}

fn metrics(status: ProbeStatus) -> Observation {
    Observation::new(ProbeId::new("host.metrics"), host(), status)
}

async fn controller(store: SqliteStore) -> Controller {
    let mut controller = Controller::new(config(), store).await.expect("controller");
    controller
        .register_agent(&RegisterRequest::new(
            "lab",
            "node-a",
            CapabilitySet::from_iter(["host.metrics"]),
        ))
        .await
        .expect("agent");
    controller
}

#[tokio::test]
async fn reset_keeps_history_but_does_not_restore_old_state_or_accept_delayed_old_measurements() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    let mut before = controller(store.clone()).await;
    let failures: Vec<_> = (0..5).map(|_| metrics(ProbeStatus::Failed)).collect();
    before.ingest_observations(&failures).await.expect("failures");
    assert_eq!(before.engine().state(host()).unwrap().overall, Health::Unavailable);
    let incident = Incident::open("host:node-a", Severity::Critical);
    store.save_incident("lab", &incident).await.expect("incident");
    drop(before);

    let reset = store.reset_state("lab").await.expect("reset");
    assert_eq!(reset.entities, 1);
    assert_eq!(reset.incidents, 1);
    assert_eq!(reset.pending_notifications, 1);
    assert!(store.load_entity_states("lab").await.unwrap().is_empty());
    assert!(store.load_state_streams("lab").await.unwrap().is_empty());
    assert_eq!(store.observation_count(host()).await.unwrap(), 5);
    assert!(store.load_active_incidents("lab").await.unwrap().is_empty());
    assert!(store.load_resumable_incidents("lab").await.unwrap().is_empty());
    assert!(store.notification_candidates("lab").await.unwrap().is_empty());
    let historical = store.load_incident(&incident.id.to_string()).await.unwrap().unwrap();
    assert_eq!(historical.status, IncidentStatus::Reset);
    assert_eq!(historical.ended_at, Some(reset.reset_at));
    assert_eq!(historical.timeline.last().unwrap().kind, "state_reset");
    assert!(sentinel::notification::notifications_for_incident(&historical).is_empty());
    assert_eq!(store.state_reset_at("lab").await.unwrap(), Some(reset.reset_at));

    let mut after = controller(store.clone()).await;
    assert!(after.engine().state(host()).is_none());
    let mut delayed = failures[0].clone();
    delayed.id = ObservationId::new();
    after
        .ingest_observations(&[delayed])
        .await
        .expect("delayed observation");
    assert!(after.engine().state(host()).is_none());
    assert!(after.diagnose().await.unwrap().is_empty());
    after
        .ingest_observations(&[metrics(ProbeStatus::Ok), metrics(ProbeStatus::Ok)])
        .await
        .expect("fresh readings");
    assert_eq!(after.engine().state(host()).unwrap().overall, Health::Healthy);
    drop(after);
    let restarted = controller(store.clone()).await;
    assert_eq!(restarted.engine().state(host()).unwrap().overall, Health::Healthy);
    assert_eq!(store.observation_count(host()).await.unwrap(), 8);
}

#[tokio::test]
async fn pre_reset_observations_with_allowed_future_skew_are_not_replayed() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    let mut before = controller(store.clone()).await;
    let mut future = metrics(ProbeStatus::Failed);
    future.finished_at += chrono::Duration::seconds(3);
    before
        .ingest_observations(std::slice::from_ref(&future))
        .await
        .expect("skewed observation");
    drop(before);
    let reset = store.reset_state("lab").await.expect("reset");
    assert!(future.finished_at > reset.reset_at);
    assert!(store
        .latest_observations_since(host(), Some(reset.reset_at))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.latest_observations(host()).await.unwrap(), vec![future]);
    let after = controller(store).await;
    assert!(after.engine().state(host()).is_none());
}

#[tokio::test]
async fn reset_affects_only_the_configured_environment_and_can_be_repeated() {
    let store = SqliteStore::open_in_memory().await.expect("store");
    let mut before = controller(store.clone()).await;
    before
        .ingest_observations(&[metrics(ProbeStatus::Ok), metrics(ProbeStatus::Ok)])
        .await
        .unwrap();
    store.ensure_environment("other").await.unwrap();
    let other = ManagedEntity::new("other", EntityType::Host, "node-a");
    store.save_entity(&other).await.unwrap();
    let mut state = EntityState::unknown(other.id);
    state.set_component(StateComponent::Host, ComponentState::new(Health::Healthy));
    store.save_entity_state(&state).await.unwrap();
    let incident = Incident::open("cause:other", Severity::Critical);
    store.save_incident("other", &incident).await.unwrap();
    drop(before);
    let first = store.reset_state("lab").await.unwrap();
    let second = store.reset_state("lab").await.unwrap();
    assert_eq!(second.entities, 0);
    assert_eq!(second.incidents, 0);
    assert_eq!(second.pending_notifications, 0);
    assert!(second.reset_at >= first.reset_at);
    assert_eq!(
        store.load_entity_states("other").await.unwrap()[&other.id].overall,
        Health::Healthy
    );
    assert!(store.state_reset_at("other").await.unwrap().is_none());
    assert_eq!(store.load_active_incidents("other").await.unwrap(), vec![incident]);
    assert_eq!(store.notification_candidates("other").await.unwrap().len(), 1);
}

#[tokio::test]
async fn reset_closes_every_unresolved_status_and_cancels_old_notifications_without_erasing_delivery_history() {
    use sentinel::notification::NotificationRecord;

    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_environment("lab").await.unwrap();
    let mut incidents = Vec::new();
    for status in [
        IncidentStatus::Open,
        IncidentStatus::Acknowledged,
        IncidentStatus::Recovering,
        IncidentStatus::Suppressed,
        IncidentStatus::Resolved,
    ] {
        let mut incident = Incident::open(format!("cause:{status}"), Severity::Critical);
        incident.status = status;
        if status == IncidentStatus::Resolved {
            incident.resolve();
        }
        store.save_incident("lab", &incident).await.unwrap();
        incidents.push(incident);
    }
    let sent = NotificationRecord {
        incident_id: incidents[0].id.to_string(),
        provider: "slack".into(),
        deduplication_key: "delivered-before-reset".into(),
        sent_at: sentinel::time::now(),
    };
    store.save_notification(&sent).await.unwrap();
    assert_eq!(store.notification_candidates("lab").await.unwrap().len(), 4);
    let reset = store.reset_state("lab").await.unwrap();
    assert_eq!(reset.incidents, 4);
    assert_eq!(reset.pending_notifications, 4);
    assert!(store.notification_candidates("lab").await.unwrap().is_empty());
    assert!(store.load_active_incidents("lab").await.unwrap().is_empty());
    // Recently resolved incidents must not be resumed across an update either.
    assert!(store.load_resumable_incidents("lab").await.unwrap().is_empty());
    assert_eq!(store.load_notifications("lab").await.unwrap(), vec![sent]);
    assert_eq!(store.load_incidents("lab", 100).await.unwrap().len(), 5);
    for incident in &incidents[..4] {
        let historical = store.load_incident(&incident.id.to_string()).await.unwrap().unwrap();
        assert_eq!(historical.status, IncidentStatus::Reset);
        assert_eq!(historical.ended_at, Some(reset.reset_at));
        assert_eq!(historical.timeline.last().unwrap().kind, "state_reset");
    }
    assert_eq!(
        store.load_incident(&incidents[4].id.to_string()).await.unwrap(),
        Some(incidents[4].clone())
    );
}

#[tokio::test]
async fn a_fault_observed_after_reset_opens_a_new_incident_even_with_the_same_cause() {
    use sentinel::dependency::DependencyGraph;
    use sentinel::diagnosis::{kind, Confidence, Diagnosis};
    use sentinel::incident::IncidentEngine;
    use std::collections::BTreeMap;

    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_environment("lab").await.unwrap();
    store
        .save_entity(&ManagedEntity::new("lab", EntityType::Host, "node-a"))
        .await
        .unwrap();
    let diagnosis = Diagnosis::new(kind::SSH_SERVICE_FAILURE, "test.rule", Confidence::High).affecting([host()]);
    let mut old_engine = IncidentEngine::new();
    let original = old_engine
        .reconcile(
            std::slice::from_ref(&diagnosis),
            &DependencyGraph::new(),
            &BTreeMap::new(),
        )
        .opened
        .remove(0);
    store.save_incident("lab", &original).await.unwrap();
    store.reset_state("lab").await.unwrap();
    let mut fresh_engine = IncidentEngine::new();
    fresh_engine.seed(store.load_resumable_incidents("lab").await.unwrap());
    let update = fresh_engine.reconcile(&[diagnosis], &DependencyGraph::new(), &BTreeMap::new());
    assert_eq!(update.opened.len(), 1);
    assert_eq!(update.opened[0].fingerprint, original.fingerprint);
    assert_ne!(update.opened[0].id, original.id);
    store.save_incident("lab", &update.opened[0]).await.unwrap();
    assert_eq!(store.load_resumable_incidents("lab").await.unwrap(), update.opened);
}

#[tokio::test]
async fn reset_failure_rolls_back_state_incidents_notifications_and_the_cutoff_together() {
    let store = SqliteStore::open_in_memory().await.unwrap();
    let mut before = controller(store.clone()).await;
    before
        .ingest_observations(&[metrics(ProbeStatus::Ok), metrics(ProbeStatus::Ok)])
        .await
        .unwrap();
    let incident = Incident::open("cause:node-a", Severity::Critical);
    store.save_incident("lab", &incident).await.unwrap();
    drop(before);
    sqlx::query("CREATE TRIGGER refuse_reset BEFORE UPDATE ON incidents WHEN NEW.status = 'reset' BEGIN SELECT RAISE(ABORT, 'reset refused'); END")
        .execute(store.pool()).await.unwrap();
    assert!(store.reset_state("lab").await.is_err());
    assert!(store.state_reset_at("lab").await.unwrap().is_none());
    assert_eq!(
        store.load_entity_states("lab").await.unwrap()[&host()].overall,
        Health::Healthy
    );
    assert!(!store.load_state_streams("lab").await.unwrap().is_empty());
    assert_eq!(
        store.load_incident(&incident.id.to_string()).await.unwrap(),
        Some(incident)
    );
    assert_eq!(store.notification_candidates("lab").await.unwrap().len(), 1);
}
