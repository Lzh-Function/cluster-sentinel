//! Failure evidence disappearing must never be interpreted as recovery.
use async_trait::async_trait;
use sentinel::config::{Config, ProbeSchedule};
use sentinel::controller::Controller;
use sentinel::diagnosis::kind;
use sentinel::entity::{EntityId, EntityKey, EntityType};
use sentinel::incident::{IncidentStatus, IncidentUpdate};
use sentinel::notification::{
    Deduplicator, MaintenanceWindows, Notification, NotificationProvider, ProviderError, Trigger,
};
use sentinel::observation::{Observation, ProbeStatus};
use sentinel::persistence::SqliteStore;
use sentinel::probes::ProbeId;
use sentinel::state::{Health, StateComponent};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

const NETWORK: &str = sentinel::probes::network::PROBE_ID;
fn host(name: &str) -> EntityId {
    EntityKey::new("lab", EntityType::Host, name).entity_id()
}
fn config() -> Config {
    let mut config = Config::from_toml("config_version = 1\nenvironment = \"lab\"\n[controller]\nobserve = false\n[[entities]]\ntype = \"host\"\nname = \"target\"\n[[entities]]\ntype = \"host\"\nname = \"a\"\n[[entities]]\ntype = \"host\"\nname = \"b\"\n[[entities]]\ntype = \"host\"\nname = \"c\"\n", std::path::Path::new("test.toml")).unwrap();
    config.notification.min_interval = Duration::ZERO;
    config
}
async fn controller() -> Controller {
    let mut c = Controller::new(config(), SqliteStore::open_in_memory().await.unwrap())
        .await
        .unwrap();
    c.discover_once().await.unwrap();
    c
}
fn network(observer: &str, good: bool, age: i64) -> Observation {
    let at = sentinel::time::now() - chrono::Duration::seconds(age);
    Observation::new(
        ProbeId::new(NETWORK),
        host("target"),
        if good { ProbeStatus::Ok } else { ProbeStatus::Timeout },
    )
    .with_observer(host(observer))
    .with_times(at, at)
    .with_payload(serde_json::json!({"host_responded": good, "outcome": if good { "connected" } else { "timed_out" }}))
}
async fn report(c: &mut Controller, good: bool, age: i64) {
    for _ in 0..3 {
        c.ingest_observations(&[network("a", good, age), network("b", good, age)])
            .await
            .unwrap();
    }
}
fn cadence(c: &mut Controller, seconds: u64) {
    c.config_mut().probes.0.insert(
        NETWORK.into(),
        ProbeSchedule {
            interval: Some(Duration::from_secs(seconds)),
            ..Default::default()
        },
    );
}
struct Recording {
    failed: AtomicBool,
    sent: tokio::sync::Mutex<Vec<Notification>>,
}
#[async_trait]
impl NotificationProvider for Recording {
    fn name(&self) -> &str {
        "test"
    }
    async fn send(&self, n: &Notification) -> Result<(), ProviderError> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(ProviderError::Unreachable {
                destination: "test".into(),
                detail: "offline".into(),
            });
        }
        self.sent.lock().await.push(n.clone());
        Ok(())
    }
}
fn recorder() -> Arc<Recording> {
    Arc::new(Recording {
        failed: AtomicBool::new(false),
        sent: Default::default(),
    })
}
async fn notify(c: &mut Controller, update: &IncidentUpdate, p: &Arc<Recording>, d: &mut Deduplicator) {
    c.notify(
        update,
        &[p.clone() as Arc<dyn NotificationProvider>],
        d,
        &MaintenanceWindows::new(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_powered_off_node_never_resolves_when_all_evidence_expires_even_after_restart() {
    let mut c = controller().await;
    cadence(&mut c, 300);
    report(&mut c, false, 300).await;
    let (_, opened) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(opened.opened.len(), 1);
    let p = recorder();
    let mut d = Deduplicator::new();
    notify(&mut c, &opened, &p, &mut d).await;
    cadence(&mut c, 5); // Same old evidence is now outside its freshness window.
    for _ in 0..3 {
        let (diagnoses, update) = c.diagnose_and_correlate().await.unwrap();
        assert!(diagnoses.is_empty());
        assert!(update.resolved.is_empty() && update.recovering.is_empty());
        notify(&mut c, &update, &p, &mut d).await;
    }
    assert_eq!(c.engine().state(host("target")).unwrap().overall, Health::Unknown);
    assert_eq!(
        c.incident_engine().active().next().unwrap().status,
        IncidentStatus::Open
    );
    let mut restarted = Controller::new(c.config().clone(), c.store().clone()).await.unwrap();
    let (_, update) = restarted.diagnose_and_correlate().await.unwrap();
    assert!(update.resolved.is_empty() && update.recovering.is_empty());
    assert_eq!(
        p.sent.lock().await.iter().map(|n| n.trigger).collect::<Vec<_>>(),
        vec![Trigger::Opened]
    );
}

#[tokio::test]
async fn losing_the_second_opinion_does_not_resolve_the_remaining_timeout() {
    let mut c = controller().await;
    cadence(&mut c, 300);
    report(&mut c, false, 300).await;
    c.diagnose_and_correlate().await.unwrap();
    cadence(&mut c, 5);
    c.ingest_observations(&[network("a", false, 0)]).await.unwrap();
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert!(update.resolved.is_empty() && update.recovering.is_empty());
    assert_eq!(c.incident_engine().active().count(), 1);
}

#[tokio::test]
async fn fresh_recovery_resolves_once_and_a_second_outage_and_recovery_are_both_announced() {
    let mut c = controller().await;
    let p = recorder();
    let mut d = Deduplicator::new();
    let mut id = None;
    for _ in 0..2 {
        report(&mut c, false, 0).await;
        let (_, update) = c.diagnose_and_correlate().await.unwrap();
        let current = c.incident_engine().active().next().unwrap().id;
        assert_eq!(
            *id.get_or_insert(current),
            current,
            "a quick recurrence reopens the incident"
        );
        notify(&mut c, &update, &p, &mut d).await;
        report(&mut c, true, 0).await;
        let (_, update) = c.diagnose_and_correlate().await.unwrap();
        assert_eq!(update.resolved.len(), 1);
        notify(&mut c, &update, &p, &mut d).await;
        let (_, unchanged) = c.diagnose_and_correlate().await.unwrap();
        notify(&mut c, &unchanged, &p, &mut d).await;
    }
    assert_eq!(
        p.sent.lock().await.iter().map(|n| n.trigger).collect::<Vec<_>>(),
        vec![Trigger::Opened, Trigger::Resolved, Trigger::Opened, Trigger::Resolved]
    );
}

#[tokio::test]
async fn a_new_healthy_quorum_can_replace_reassigned_observers() {
    let mut c = controller().await;
    cadence(&mut c, 300);
    report(&mut c, false, 300).await;
    c.diagnose_and_correlate().await.unwrap();
    cadence(&mut c, 5);
    for _ in 0..3 {
        c.ingest_observations(&[network("a", true, 0), network("c", true, 0)])
            .await
            .unwrap();
    }
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.resolved.len(), 1);
}

#[tokio::test]
async fn a_failed_resolution_delivery_is_retried_after_a_controller_restart() {
    let mut c = controller().await;
    report(&mut c, false, 0).await;
    c.diagnose_and_correlate().await.unwrap();
    report(&mut c, true, 0).await;
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.resolved.len(), 1);
    let p = recorder();
    let mut d = Deduplicator::new();
    p.failed.store(true, Ordering::SeqCst);
    notify(&mut c, &update, &p, &mut d).await;
    let mut restarted = Controller::new(c.config().clone(), c.store().clone()).await.unwrap();
    p.failed.store(false, Ordering::SeqCst);
    let (_, update) = restarted.diagnose_and_correlate().await.unwrap();
    notify(&mut restarted, &update, &p, &mut d).await;
    notify(&mut restarted, &IncidentUpdate::default(), &p, &mut d).await;
    assert_eq!(
        p.sent.lock().await.iter().map(|n| n.trigger).collect::<Vec<_>>(),
        vec![Trigger::Resolved]
    );
}

#[tokio::test]
async fn replaying_a_single_failure_does_not_meet_the_failure_threshold_even_after_restart() {
    let mut c = controller().await;
    report(&mut c, true, 0).await;
    let failure = network("a", false, 0);
    c.ingest_observations(std::slice::from_ref(&failure)).await.unwrap();
    let mut restarted = Controller::new(c.config().clone(), c.store().clone()).await.unwrap();
    for _ in 0..4 {
        restarted
            .ingest_observations(std::slice::from_ref(&failure))
            .await
            .unwrap();
    }
    assert_eq!(
        restarted
            .engine()
            .state(host("target"))
            .unwrap()
            .component(StateComponent::Network),
        Health::Healthy
    );
    restarted.ingest_observations(&[network("a", false, 0)]).await.unwrap();
    assert_eq!(
        restarted
            .engine()
            .state(host("target"))
            .unwrap()
            .component(StateComponent::Network),
        Health::Degraded
    );
}

#[tokio::test]
async fn delayed_old_successes_cannot_recover_a_current_outage() {
    let mut c = controller().await;
    report(&mut c, false, 0).await;
    c.diagnose_and_correlate().await.unwrap();
    report(&mut c, true, 300).await;
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert!(update.resolved.is_empty());
    assert_eq!(
        c.engine()
            .state(host("target"))
            .unwrap()
            .component(StateComponent::Network),
        Health::Unavailable
    );
}

#[tokio::test]
async fn failing_to_save_state_rolls_back_evidence_and_leaves_live_state_unchanged() {
    let mut c = controller().await;
    report(&mut c, true, 0).await;
    let before = c.store().observation_count(host("target")).await.unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_state BEFORE UPDATE ON entity_states BEGIN SELECT RAISE(ABORT, 'test state failure'); END",
    )
    .execute(c.store().pool())
    .await
    .unwrap();
    assert!(c.ingest_observations(&[network("a", false, 0)]).await.is_err());
    assert_eq!(c.store().observation_count(host("target")).await.unwrap(), before);
    assert_eq!(
        c.engine()
            .state(host("target"))
            .unwrap()
            .component(StateComponent::Network),
        Health::Healthy
    );
}

#[tokio::test]
async fn failing_to_save_notification_candidates_does_not_consume_an_incident_opening() {
    let mut c = controller().await;
    report(&mut c, false, 0).await;
    sqlx::query("CREATE TRIGGER fail_candidate BEFORE INSERT ON notification_candidates BEGIN SELECT RAISE(ABORT, 'test notification failure'); END").execute(c.store().pool()).await.unwrap();
    assert!(c.diagnose_and_correlate().await.is_err());
    assert_eq!(c.incident_engine().active().count(), 0);
    assert!(c.store().load_active_incidents("lab").await.unwrap().is_empty());
    sqlx::query("DROP TRIGGER fail_candidate")
        .execute(c.store().pool())
        .await
        .unwrap();
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.opened.len(), 1);
}

#[tokio::test]
async fn an_expired_scheduler_fault_cannot_be_cleared_by_healthy_host_reports() {
    let mut c = controller().await;
    for _ in 0..3 {
        c.ingest_observations(&[Observation::new(
            ProbeId::new(sentinel::probes::sentinel_rpc::PROBE_ID),
            host("target"),
            ProbeStatus::Ok,
        )])
        .await
        .unwrap();
    }
    report(&mut c, true, 0).await;
    let at = sentinel::time::now() - chrono::Duration::seconds(300);
    let drained = Observation::new(ProbeId::new("slurm.node"), host("target"), ProbeStatus::Degraded).with_times(at, at)
        .with_payload(serde_json::json!({"drained": true, "schedulable": false, "reason": "maintenance", "state": "IDLE+DRAIN", "responding": true}));
    c.ingest_observations(&[drained]).await.unwrap();
    let (diagnoses, update) = c.diagnose_and_correlate().await.unwrap();
    assert!(
        diagnoses.iter().any(|d| d.is(kind::SLURM_ONLY_DEGRADATION)),
        "{diagnoses:?}"
    );
    assert_eq!(update.opened.len(), 1);
    c.config_mut().probes.0.insert(
        "slurm.node".into(),
        ProbeSchedule {
            interval: Some(Duration::from_secs(5)),
            ..Default::default()
        },
    );
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert!(update.resolved.is_empty() && update.recovering.is_empty());
    c.ingest_observations(&[
        Observation::new(ProbeId::new("slurm.node"), host("target"), ProbeStatus::Ok)
            .with_payload(serde_json::json!({"schedulable": true, "state": "IDLE", "responding": true})),
    ])
    .await
    .unwrap();
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.resolved.len(), 1);
}

#[tokio::test]
async fn a_new_incident_cancels_an_older_undelivered_resolution_for_the_same_cause() {
    let mut c = controller().await;
    report(&mut c, false, 0).await;
    c.diagnose_and_correlate().await.unwrap();
    report(&mut c, true, 0).await;
    let (_, recovery) = c.diagnose_and_correlate().await.unwrap();
    let previous = &recovery.resolved[0];
    let mut next = sentinel::incident::Incident::open(&previous.fingerprint, previous.severity);
    for diagnosis in &previous.diagnoses {
        next.add_diagnosis(diagnosis.clone());
    }
    c.store().save_incident("lab", &next).await.unwrap();
    let p = recorder();
    let mut d = Deduplicator::new();
    notify(
        &mut c,
        &IncidentUpdate {
            opened: vec![next],
            ..Default::default()
        },
        &p,
        &mut d,
    )
    .await;
    assert_eq!(
        p.sent.lock().await.iter().map(|n| n.trigger).collect::<Vec<_>>(),
        vec![Trigger::Opened]
    );
}

#[tokio::test]
async fn recurrence_identity_survives_a_restart_after_resolution() {
    let mut c = controller().await;
    report(&mut c, false, 0).await;
    let (_, opened) = c.diagnose_and_correlate().await.unwrap();
    let id = opened.opened[0].id;
    report(&mut c, true, 0).await;
    c.diagnose_and_correlate().await.unwrap();
    let mut restarted = Controller::new(c.config().clone(), c.store().clone()).await.unwrap();
    report(&mut restarted, false, 0).await;
    let (_, update) = restarted.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.updated[0].id, id);
    assert_eq!(update.updated[0].status, IncidentStatus::Open);
}

#[tokio::test]
async fn loading_active_incidents_is_not_limited_by_recent_resolved_incidents() {
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_environment("lab").await.unwrap();
    sqlx::query("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i < 1005) INSERT INTO incidents (id, environment, fingerprint, status, severity, started_at, ended_at, updated_at) SELECT CAST(i AS TEXT), 'lab', CAST(i AS TEXT), 'resolved', 'warning', '2026-01-02T00:00:00.000000Z', '2026-01-02T00:01:00.000000Z', '2026-01-02T00:01:00.000000Z' FROM n").execute(store.pool()).await.unwrap();
    let mut active = sentinel::incident::Incident::open("old-active", sentinel::incident::Severity::Critical);
    active.started_at = sentinel::time::now() - chrono::Duration::days(10000);
    store.save_incident("lab", &active).await.unwrap();
    assert_eq!(
        store
            .load_active_incidents("lab")
            .await
            .unwrap()
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        vec![active.id]
    );
}

#[tokio::test]
async fn controller_votes_use_the_discovery_cadence_instead_of_the_peer_cadence() {
    let mut c = controller().await;
    let own = c.observer_entity().unwrap();
    let name = hostname::get().unwrap().to_string_lossy().to_string();
    let entity = sentinel::entity::ManagedEntity::new("lab", EntityType::Host, name);
    assert_eq!(entity.id, own);
    c.store()
        .save_inventory(&{
            let mut inventory = c.store().load_inventory("lab").await.unwrap();
            inventory.insert_entity(entity);
            inventory
        })
        .await
        .unwrap();
    for _ in 0..3 {
        let mut older_controller_vote = network("b", false, 60);
        older_controller_vote.observer_entity = Some(own);
        c.ingest_observations(&[older_controller_vote, network("a", false, 0)])
            .await
            .unwrap();
    }
    let diagnoses = c.diagnose().await.unwrap();
    assert!(diagnoses.iter().any(|d| d.is(kind::HOST_UNREACHABLE)), "{diagnoses:?}");
}

#[tokio::test]
async fn separate_notification_passes_still_honor_the_minimum_send_interval() {
    let mut c = controller().await;
    c.config_mut().notification.min_interval = Duration::from_millis(25);
    let p = recorder();
    let mut d = Deduplicator::new();
    let first = sentinel::incident::Incident::open("first", sentinel::incident::Severity::Critical);
    let second = sentinel::incident::Incident::open("second", sentinel::incident::Severity::Critical);
    let started = std::time::Instant::now();
    notify(
        &mut c,
        &IncidentUpdate {
            opened: vec![first],
            ..Default::default()
        },
        &p,
        &mut d,
    )
    .await;
    notify(
        &mut c,
        &IncidentUpdate {
            opened: vec![second],
            ..Default::default()
        },
        &p,
        &mut d,
    )
    .await;
    assert!(started.elapsed() >= Duration::from_millis(20));
    assert_eq!(p.sent.lock().await.len(), 2);
}

#[tokio::test]
async fn an_unrelated_unsupported_probe_does_not_prevent_proven_reachability_recovery() {
    let mut c = controller().await;
    let unsupported = Observation::new(
        ProbeId::new(sentinel::probes::gpu::PROBE_ID),
        host("target"),
        ProbeStatus::Unsupported,
    );
    c.ingest_observations(&[unsupported]).await.unwrap();
    report(&mut c, false, 0).await;
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.opened.len(), 1);
    report(&mut c, true, 0).await;
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.resolved.len(), 1);
    assert_eq!(
        c.engine()
            .state(host("target"))
            .unwrap()
            .component(StateComponent::Accelerator),
        Health::Unknown
    );
}

#[tokio::test]
async fn recovery_after_expiry_still_requires_two_good_rounds_per_observer() {
    let mut c = controller().await;
    cadence(&mut c, 300);
    report(&mut c, false, 300).await;
    c.diagnose_and_correlate().await.unwrap();
    cadence(&mut c, 5);
    c.diagnose_and_correlate().await.unwrap();
    c.ingest_observations(&[network("a", true, 0), network("b", true, 0)])
        .await
        .unwrap();
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert!(update.resolved.is_empty() && update.recovering.is_empty());
    c.ingest_observations(&[network("a", true, 0), network("b", true, 0)])
        .await
        .unwrap();
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.resolved.len(), 1);
}

#[tokio::test]
async fn future_dated_successes_cannot_recover_an_outage_or_poison_later_valid_observations() {
    let mut c = controller().await;
    report(&mut c, false, 0).await;
    c.diagnose_and_correlate().await.unwrap();
    report(&mut c, true, -3600).await;
    let (diagnoses, update) = c.diagnose_and_correlate().await.unwrap();
    assert!(diagnoses.iter().any(|d| d.is(kind::HOST_UNREACHABLE)));
    assert!(update.resolved.is_empty());
    assert_eq!(
        c.engine()
            .state(host("target"))
            .unwrap()
            .component(StateComponent::Network),
        Health::Unavailable
    );
    report(&mut c, true, 0).await;
    let (_, update) = c.diagnose_and_correlate().await.unwrap();
    assert_eq!(update.resolved.len(), 1);
}

#[tokio::test]
async fn conflicting_duplicate_ids_cannot_create_state_for_unstored_observations() {
    let mut c = controller().await;
    let first = network("a", false, 0);
    let mut conflict = first.clone();
    conflict.target_entity = host("c");
    conflict.status = ProbeStatus::Ok;
    c.ingest_observations(&[first.clone(), conflict]).await.unwrap();
    assert!(c.engine().state(host("c")).is_none());
    assert_eq!(c.store().observation_count(host("target")).await.unwrap(), 1);
    assert_eq!(c.store().observation_by_id(first.id).await.unwrap().unwrap(), first);
}

#[tokio::test]
async fn equal_timestamps_keep_state_and_latest_evidence_in_arrival_order() {
    let mut c = controller().await;
    let at = sentinel::time::now();
    let mut failed =
        Observation::new(ProbeId::new("slurm.node"), host("target"), ProbeStatus::Failed).with_times(at, at);
    failed.id = sentinel::observation::ObservationId::from_uuid(uuid::Uuid::from_u128(u128::MAX));
    let mut good = failed.clone();
    good.id = sentinel::observation::ObservationId::from_uuid(uuid::Uuid::from_u128(1));
    good.status = ProbeStatus::Ok;
    c.ingest_observations(&[failed, good.clone()]).await.unwrap();
    assert_eq!(c.store().latest_observations(host("target")).await.unwrap()[0], good);
    assert_eq!(
        c.engine()
            .state(host("target"))
            .unwrap()
            .component(StateComponent::Scheduler),
        Health::Healthy
    );
}
