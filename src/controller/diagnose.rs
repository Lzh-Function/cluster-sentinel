//! Running the diagnosis pass.
//!
//! Diagnosis reads what is already stored — inventory, state, observations —
//! and writes conclusions. It never probes anything itself, which is what makes
//! a diagnosis reproducible: the same stored evidence always yields the same
//! answer, and an operator can re-run it later and check.

use std::collections::{BTreeMap, HashMap};

use crate::diagnosis::{Diagnosis, DiagnosisContext, ObservationIndex};
use crate::entity::EntityId;
use crate::incident::IncidentUpdate;
use crate::persistence::StoreError;
use crate::state::{Classification, EntityState, Health};

use super::Controller;

/// How many of its own intervals an observation may lag before it stops being
/// evidence about now.
///
/// Missing one round is a hiccup; missing four is a probe that has stopped
/// running -- because the observer was reassigned, or is itself gone. Its last
/// answer must not keep voting. Four leaves room for jitter and a slow cycle
/// while still expiring a reachability answer in twenty seconds.
const STALE_AFTER_INTERVALS: u32 = 4;

/// How long each probe's answer stays evidence.
///
/// Derived from the probe's own cadence, because staleness is relative: a
/// reachability answer from a minute ago is worthless, and a Slurm node view
/// from a minute ago is current. A count-based window cannot express that --
/// which is what made a stopped host look like a broken path, since a
/// reassigned observer's last "reachable" stayed the newest one it had
/// forever.
fn freshness_horizons(config: &crate::config::Config) -> HashMap<String, chrono::Duration> {
    let mut horizons = HashMap::new();
    for entry in crate::probes::catalog::catalog() {
        let mut definition = entry.definition.clone();
        config.probes.apply(&mut definition);
        let window = definition.interval * STALE_AFTER_INTERVALS;
        horizons.insert(
            definition.id.as_str().to_string(),
            chrono::Duration::from_std(window).unwrap_or_else(|_| chrono::Duration::hours(1)),
        );
    }
    horizons
}

// Controller observations run during discovery; peer probes run on their own
// schedule. Using a five-second peer cadence for both expires the controller's
// vote between inventory sweeps.
fn horizon_for(
    probe: &str,
    observer: Option<EntityId>,
    controller: Option<EntityId>,
    horizons: &HashMap<String, chrono::Duration>,
    fallback: chrono::Duration,
) -> chrono::Duration {
    let probe_horizon = horizons.get(probe).copied().unwrap_or(fallback);
    if observer.is_some() && observer == controller {
        probe_horizon.max(fallback)
    } else {
        probe_horizon
    }
}

impl Controller {
    /// Run every diagnosis rule against the current picture.
    pub async fn diagnose(&self) -> Result<Vec<Diagnosis>, StoreError> {
        let environment = self.config().environment.clone();
        let inventory = self.store().load_inventory(&environment).await?;

        let horizons = freshness_horizons(self.config());
        let fallback = chrono::Duration::from_std(self.config().controller.inventory_interval * STALE_AFTER_INTERVALS)
            .unwrap_or_else(|_| chrono::Duration::hours(1));
        let observer = self.observer_entity();
        let mut current_engine = self.engine().clone();
        current_engine.expire(crate::time::now(), |s| {
            horizon_for(s.probe.as_str(), s.observer, observer, &horizons, fallback)
        });

        // Prefer the live state engine over the database: it is what the
        // observations in this cycle have just updated, and reloading would
        // race with the write.
        let mut states: HashMap<EntityId, EntityState> =
            current_engine.states().map(|s| (s.entity, s.clone())).collect();
        if states.is_empty() {
            states = self
                .store()
                .load_entity_states(&environment)
                .await?
                .into_iter()
                .collect();
        }

        // Observations that are no longer evidence about now are left out
        // rather than weighed: a rule cannot tell a stale answer from a
        // current one, and every rule here treats what it is given as the
        // present.
        let now = crate::time::now();

        let mut observations = ObservationIndex::new();
        for entity in inventory.entities() {
            for observation in self
                .store()
                .latest_observations_since(entity.id, current_engine.observation_cutoff())
                .await?
            {
                if !current_engine.probe_enabled(entity.id, &observation.probe_id) {
                    continue;
                }
                let horizon = horizon_for(
                    observation.probe_id.as_str(),
                    observation.observer_entity,
                    observer,
                    &horizons,
                    fallback,
                );
                if now - observation.finished_at <= horizon {
                    observations.insert(observation);
                }
            }
        }

        let context = DiagnosisContext {
            environment: &environment,
            inventory: &inventory,
            states: &states,
            observations: &observations,
        };

        Ok(self.diagnosis_engine().diagnose(&context))
    }

    /// Run diagnosis and apply the classifications it implies to entity state.
    ///
    /// A classification is the short machine-readable label an operator sees
    /// next to an entity. It comes from the diagnosis rather than from a probe,
    /// because only a rule has looked at enough evidence to justify one.
    ///
    /// Every entity is rewritten, including the ones this cycle said nothing
    /// about: a diagnosis that no longer fires must take its label with it.
    pub async fn diagnose_and_classify(&mut self) -> Result<Vec<Diagnosis>, StoreError> {
        let previous = self.engine.clone();
        let result = self.diagnose_and_classify_inner().await;
        if result.is_err() {
            self.engine = previous;
        }
        result
    }

    async fn diagnose_and_classify_inner(&mut self) -> Result<Vec<Diagnosis>, StoreError> {
        let horizons = freshness_horizons(self.config());
        let fallback = chrono::Duration::from_std(self.config().controller.inventory_interval * STALE_AFTER_INTERVALS)
            .unwrap_or_else(|_| chrono::Duration::hours(1));
        let observer = self.observer_entity();
        let transitions = self.engine_mut().expire(crate::time::now(), |s| {
            horizon_for(s.probe.as_str(), s.observer, observer, &horizons, fallback)
        });
        let diagnoses = self.diagnose().await?;

        let mut implied: BTreeMap<EntityId, Vec<Classification>> = BTreeMap::new();
        for diagnosis in &diagnoses {
            let Some(classification) = crate::diagnosis::classification_for(diagnosis) else {
                continue;
            };
            for entity in &diagnosis.affected_entities {
                implied
                    .entry(*entity)
                    .or_default()
                    .push(Classification::new(classification));
            }
        }

        let entities: Vec<EntityId> = self.engine().states().map(|s| s.entity).collect();
        for entity in entities {
            let classifications = implied.remove(&entity).unwrap_or_default();
            if let Some(state) = self.engine_mut().state_mut(entity) {
                state.set_classifications(classifications);
            }
        }

        self.store()
            .save_engine(&self.config().environment, self.engine(), &transitions)
            .await?;

        Ok(diagnoses)
    }

    /// Diagnose, classify, then fold the result into incidents.
    ///
    /// The order matters: incidents are built from diagnoses, and diagnoses
    /// from state, so each stage sees a settled picture from the one before.
    pub async fn diagnose_and_correlate(&mut self) -> Result<(Vec<Diagnosis>, IncidentUpdate), StoreError> {
        let diagnoses = self.diagnose_and_classify().await?;

        let environment = self.config().environment.clone();
        let inventory = self.store().load_inventory(&environment).await?;
        let states: BTreeMap<EntityId, EntityState> = self.engine().states().map(|s| (s.entity, s.clone())).collect();

        let horizons = freshness_horizons(self.config());
        let fallback = chrono::Duration::from_std(self.config().controller.inventory_interval * STALE_AFTER_INTERVALS)
            .unwrap_or_else(|_| chrono::Duration::hours(1));
        let observer = self.observer_entity();
        let at = crate::time::now();
        let mut latest = ObservationIndex::new();
        for entity in inventory.entities() {
            for o in self
                .store()
                .latest_observations_since(entity.id, self.engine().observation_cutoff())
                .await?
            {
                if !self.engine().probe_enabled(entity.id, &o.probe_id) {
                    continue;
                }
                if at - o.finished_at
                    <= horizon_for(o.probe_id.as_str(), o.observer_entity, observer, &horizons, fallback)
                {
                    latest.insert(o);
                }
            }
        }
        let mut verdicts = BTreeMap::new();
        let diagnosed: std::collections::BTreeSet<_> = diagnoses.iter().map(crate::incident::fingerprint).collect();
        for incident in self.incidents.active().filter(|i| !diagnosed.contains(&i.fingerprint)) {
            let mut originals = Vec::new();
            for id in &incident.evidence {
                if let Some(o) = self.store().observation_by_id(*id).await? {
                    originals.push(o);
                } else {
                    originals.clear();
                    break;
                }
            }
            let roots: std::collections::BTreeSet<_> = incident.suspected_root_entities.iter().copied().collect();
            let verified = |root_only: bool| {
                if originals.is_empty() {
                    return false;
                }
                let selected: Vec<_> = originals
                    .iter()
                    .filter(|o| {
                        o.status.is_conclusive()
                            && o.probe_id.as_str() != crate::controller::registration::PROBE_BOOT
                            && (!root_only || roots.contains(&o.target_entity))
                    })
                    .collect();
                // A conceptual service can be rooted at a daemon but measured
                // through the scheduler/host. Its evidence remains mandatory.
                let selected = if selected.is_empty() {
                    originals
                        .iter()
                        .filter(|o| {
                            o.status.is_conclusive()
                                && o.probe_id.as_str() != crate::controller::registration::PROBE_BOOT
                        })
                        .collect()
                } else {
                    selected
                };
                !selected.is_empty()
                    && !diagnoses.iter().any(|d| {
                        d.affected_entities.iter().any(|e| {
                            if root_only {
                                roots.contains(e)
                            } else {
                                incident.affected_entities.contains(e) || roots.contains(e)
                            }
                        })
                    })
                    && selected.iter().all(|old| {
                        // Host unreachability is a quorum statement, not a fault
                        // of one observer's path. A new healthy quorum can replace
                        // observers that have been reassigned.
                        if old.probe_id.as_str() == crate::probes::network::PROBE_ID
                            && incident.has_diagnosis(crate::diagnosis::kind::HOST_UNREACHABLE)
                        {
                            let views = latest.all(old.target_entity, old.probe_id.as_str());
                            let remote: Vec<_> = views
                                .iter()
                                .filter(|o| {
                                    o.observer_entity.is_some()
                                        && o.observer_entity != Some(old.target_entity)
                                        && o.status.is_conclusive()
                                })
                                .collect();
                            return remote.len() >= 2
                                && remote.iter().all(|o| {
                                    o.status == crate::observation::ProbeStatus::Ok
                                        && o.finished_at > old.finished_at
                                        && self.engine().confirms_health(o)
                                });
                        }
                        latest.all(old.target_entity, old.probe_id.as_str()).iter().any(|new| {
                            // Healthy context is not a failed path: a current
                            // observer can confirm it after reassignment. Bad
                            // and degraded evidence still requires recovery
                            // from the observer that reported the fault.
                            (new.observer_entity == old.observer_entity
                                || old.status == crate::observation::ProbeStatus::Ok)
                                && self.engine().confirms_health(new)
                                && new.status == crate::observation::ProbeStatus::Ok
                                && (new.finished_at > old.finished_at
                                    || (!old.status.is_bad()
                                        && old.status != crate::observation::ProbeStatus::Degraded))
                        })
                    })
                    && incident
                        .affected_entities
                        .iter()
                        .chain(&incident.suspected_root_entities)
                        .filter(|e| !root_only || roots.contains(e))
                        .all(|e| {
                            states
                                .get(e)
                                .map(|s| {
                                    s.overall == Health::Healthy
                                        || (s.overall == Health::Unknown
                                            && selected.iter().any(|o| o.target_entity == *e))
                                })
                                .unwrap_or_else(|| {
                                    inventory
                                        .get(*e)
                                        .is_some_and(|entity| entity.entity_type == crate::entity::EntityType::Service)
                                })
                        })
            };
            verdicts.insert(incident.id, (verified(true), verified(false)));
        }
        let mut candidate = self.incidents.clone();
        let update = candidate.reconcile_verified(&diagnoses, inventory.graph(), |i| {
            verdicts.get(&i.id).copied().unwrap_or((false, false))
        });
        self.store().save_incidents(&environment, &update.all()).await?;
        self.incidents = candidate;
        Ok((diagnoses, update))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::diagnosis::kind;
    use crate::entity::{EntityKey, EntityType};
    use crate::integrations::slurm::{parser, SlurmView};
    use crate::inventory::slurm::snapshot_from_view;
    use crate::persistence::SqliteStore;
    use crate::state::classification;

    async fn controller() -> Controller {
        let config = Config {
            config_version: 1,
            environment: "lab".into(),
            ..Config::default()
        };
        Controller::new(config, SqliteStore::open_in_memory().await.expect("store"))
            .await
            .expect("controller")
    }

    fn view(nodes: &str) -> SlurmView {
        SlurmView {
            nodes: parser::parse_nodes(nodes),
            partitions: Vec::new(),
            controllers: Vec::new(),
        }
    }

    /// Give a host positive evidence that it is reachable and answering.
    async fn make_host_look_healthy(controller: &mut Controller, name: &str) {
        use crate::observation::{Observation, ProbeStatus};
        use crate::probes::ProbeId;

        let host = EntityKey::new("lab", EntityType::Host, name).entity_id();
        let mut observations = Vec::new();
        for _ in 0..3 {
            observations.push(Observation::new(
                ProbeId::new(crate::probes::network::PROBE_ID),
                host,
                ProbeStatus::Ok,
            ));
            observations.push(Observation::new(
                ProbeId::new(crate::probes::sentinel_rpc::PROBE_ID),
                host,
                ProbeStatus::Ok,
            ));
        }
        controller
            .ingest_observations(&observations)
            .await
            .expect("host evidence");
    }

    #[tokio::test]
    async fn a_healthy_cluster_yields_no_diagnoses() {
        let mut controller = controller().await;
        let view = view("NodeName=n1 State=IDLE\n");
        controller
            .ingest_snapshot(&snapshot_from_view("lab", "sched", &view))
            .await
            .expect("inventory");
        controller
            .ingest_observations(&crate::integrations::slurm::observe::observations_from_view(
                "lab", "sched", &view,
            ))
            .await
            .expect("observations");

        assert!(controller.diagnose().await.expect("diagnose").is_empty());
    }

    #[tokio::test]
    async fn a_drained_node_on_a_healthy_host_is_diagnosed_and_classified() {
        let mut controller = controller().await;
        let view = view("NodeName=n1 State=IDLE+DRAIN Reason=maintenance\n");
        controller
            .ingest_snapshot(&snapshot_from_view("lab", "sched", &view))
            .await
            .expect("inventory");
        make_host_look_healthy(&mut controller, "n1").await;
        controller
            .ingest_observations(&crate::integrations::slurm::observe::observations_from_view(
                "lab", "sched", &view,
            ))
            .await
            .expect("observations");

        let diagnoses = controller.diagnose_and_classify().await.expect("diagnose");
        assert_eq!(diagnoses.len(), 1);
        assert!(diagnoses[0].is(kind::SLURM_ONLY_DEGRADATION));

        let host = EntityKey::new("lab", EntityType::Host, "n1").entity_id();
        let state = controller.engine().state(host).expect("state");
        assert!(state.has_classification(classification::SCHEDULER_DEGRADED));
    }

    #[tokio::test]
    async fn an_unresponsive_node_on_a_healthy_host_is_a_slurmd_failure() {
        let mut controller = controller().await;
        let view = view("NodeName=n1 State=DOWN* Reason=Not responding\n");
        controller
            .ingest_snapshot(&snapshot_from_view("lab", "sched", &view))
            .await
            .expect("inventory");
        make_host_look_healthy(&mut controller, "n1").await;
        controller
            .ingest_observations(&crate::integrations::slurm::observe::observations_from_view(
                "lab", "sched", &view,
            ))
            .await
            .expect("observations");

        let diagnoses = controller.diagnose().await.expect("diagnose");
        assert_eq!(diagnoses.len(), 1);
        assert!(diagnoses[0].is(kind::SLURMD_SERVICE_FAILURE));
    }

    #[tokio::test]
    async fn an_unresponsive_node_with_no_host_evidence_is_not_blamed_on_slurmd() {
        // The controller has not probed this host, so it has no grounds to say
        // the machine is fine and only the daemon is broken.
        let mut controller = controller().await;
        let view = view("NodeName=n1 State=DOWN* Reason=Not responding\n");
        controller
            .ingest_snapshot(&snapshot_from_view("lab", "sched", &view))
            .await
            .expect("inventory");
        controller
            .ingest_observations(&crate::integrations::slurm::observe::observations_from_view(
                "lab", "sched", &view,
            ))
            .await
            .expect("observations");

        assert!(controller.diagnose().await.expect("diagnose").is_empty());
    }

    #[tokio::test]
    async fn diagnoses_carry_evidence_that_can_be_looked_up() {
        let mut controller = controller().await;
        let view = view("NodeName=n1 State=IDLE+DRAIN Reason=maintenance\n");
        controller
            .ingest_snapshot(&snapshot_from_view("lab", "sched", &view))
            .await
            .expect("inventory");
        make_host_look_healthy(&mut controller, "n1").await;
        controller
            .ingest_observations(&crate::integrations::slurm::observe::observations_from_view(
                "lab", "sched", &view,
            ))
            .await
            .expect("observations");

        let diagnoses = controller.diagnose().await.expect("diagnose");
        let diagnosis = &diagnoses[0];

        assert!(
            !diagnosis.evidence.is_empty(),
            "a diagnosis with no evidence cannot be checked"
        );
        assert!(!diagnosis.rule_id.as_str().is_empty());

        // Every cited observation must actually be retrievable.
        let host = EntityKey::new("lab", EntityType::Host, "n1").entity_id();
        let stored: Vec<_> = controller
            .store()
            .recent_observations(host, 64)
            .await
            .expect("observations")
            .into_iter()
            .map(|o| o.id)
            .collect();
        for cited in &diagnosis.evidence {
            assert!(stored.contains(cited), "cited observation {cited} is not stored");
        }
    }

    #[tokio::test]
    async fn diagnosis_is_reproducible() {
        let mut controller = controller().await;
        let view = view("NodeName=n1 State=IDLE+DRAIN Reason=maintenance\n");
        controller
            .ingest_snapshot(&snapshot_from_view("lab", "sched", &view))
            .await
            .expect("inventory");
        make_host_look_healthy(&mut controller, "n1").await;
        controller
            .ingest_observations(&crate::integrations::slurm::observe::observations_from_view(
                "lab", "sched", &view,
            ))
            .await
            .expect("observations");

        let first = controller.diagnose().await.expect("first");
        let second = controller.diagnose().await.expect("second");

        let describe = |d: &[Diagnosis]| {
            d.iter()
                .map(|d| (d.diagnosis_type.to_string(), d.summary.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(describe(&first), describe(&second));
    }
}
