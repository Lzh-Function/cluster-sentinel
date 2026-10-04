//! Folding observations into entity state.
//!
//! The engine is deliberately ignorant of what any probe measures. An
//! integration *declares* which state component its probe informs, and the
//! engine does the debouncing and roll-up. Adding a Ceph probe is a
//! registration, not a change here (SPEC.md §57).

use std::collections::{BTreeMap, HashMap};

use crate::entity::EntityId;
use crate::observation::{Observation, ProbeStatus};
use crate::probes::ProbeId;
use crate::time::Timestamp;
use serde::{Deserialize, Serialize};

use super::{ComponentState, DebouncePolicy, Debouncer, EntityState, Health, StateComponent, StateTransition};

/// How one probe's results feed into state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeMapping {
    /// Which component this probe informs.
    pub component: StateComponent,
    /// How much evidence is needed before health changes.
    pub policy: DebouncePolicy,
    /// Event probes preserve history without asserting component health.
    pub informational: bool,
}

impl ProbeMapping {
    /// A mapping with the default debounce policy.
    pub fn new(component: StateComponent) -> Self {
        Self {
            component,
            policy: DebouncePolicy::default(),
            informational: false,
        }
    }

    /// A mapping that reacts to the first observation.
    ///
    /// For probes whose result is an authoritative statement rather than a
    /// measurement: a scheduler reporting DRAIN is not a flaky ping
    /// (IMPLEMENTATION.md §70).
    pub fn immediate(component: StateComponent) -> Self {
        Self {
            component,
            policy: DebouncePolicy::immediate(),
            informational: false,
        }
    }

    pub fn informational(mut self) -> Self {
        self.informational = true;
        self
    }

    /// Builder: use a specific policy.
    pub fn with_policy(mut self, policy: DebouncePolicy) -> Self {
        self.policy = policy;
        self
    }
}

/// One independently debounced probe and observer. Persisted across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStream {
    pub entity: EntityId,
    pub probe: ProbeId,
    pub observer: Option<EntityId>,
    pub component: StateComponent,
    pub last_observation: crate::observation::ObservationId,
    pub finished_at: Timestamp,
    pub debouncer: Debouncer,
    pub state: ComponentState,
    #[serde(default)]
    pub expired: bool,
}

/// Derives [`EntityState`] from a stream of observations.
#[derive(Debug, Default, Clone)]
pub struct StateEngine {
    mappings: BTreeMap<ProbeId, ProbeMapping>,
    streams: HashMap<(EntityId, ProbeId, Option<EntityId>), StateStream>,
    states: BTreeMap<EntityId, EntityState>,
    disabled: std::collections::HashSet<(EntityId, ProbeId)>,
    observation_cutoff: Option<Timestamp>,
}

impl StateEngine {
    /// An engine that knows about no probes yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare how a probe's results map onto a state component.
    pub fn register(&mut self, probe_id: impl Into<ProbeId>, mapping: ProbeMapping) -> &mut Self {
        self.mappings.insert(probe_id.into(), mapping);
        self
    }

    /// Whether a probe has been registered.
    pub fn knows(&self, probe_id: &ProbeId) -> bool {
        self.mappings.contains_key(probe_id)
    }

    /// Whether inventory allows this probe to contribute to this entity.
    pub fn probe_enabled(&self, entity: EntityId, probe: &ProbeId) -> bool {
        !self.disabled.contains(&(entity, probe.clone()))
    }

    pub fn observation_cutoff(&self) -> Option<Timestamp> {
        self.observation_cutoff
    }

    pub fn set_observation_cutoff(&mut self, cutoff: Option<Timestamp>) {
        self.observation_cutoff = cutoff;
    }

    /// Exclude a retired probe without deleting its observations or streams.
    /// Re-enabling requires new measurements instead of reviving old health.
    pub fn set_probe_enabled(&mut self, entity: EntityId, probe: &ProbeId, enabled: bool) -> Option<StateTransition> {
        let component = self.mappings.get(probe)?.component;
        let changed = if enabled {
            self.disabled.remove(&(entity, probe.clone()))
        } else {
            self.disabled.insert((entity, probe.clone()))
        };
        if !changed {
            return None;
        }
        if enabled {
            for stream in self
                .streams
                .values_mut()
                .filter(|s| s.entity == entity && &s.probe == probe)
            {
                stream.expired = true;
                stream.state.health = Health::Unknown;
                stream.debouncer.force(Health::Unknown);
            }
        }
        if self
            .states
            .get(&entity)
            .is_some_and(|s| s.components.contains_key(&component))
        {
            self.recompute(entity, component)
        } else {
            None
        }
    }

    /// Fold one observation in, returning any state change it caused.
    ///
    /// An observation from an unregistered probe is stored as history by the
    /// caller but changes no state: an integration that has not said what its
    /// probe means must not be guessed at.
    pub fn ingest(&mut self, observation: &Observation) -> Option<StateTransition> {
        if !self.probe_enabled(observation.target_entity, &observation.probe_id)
            || self
                .observation_cutoff
                .is_some_and(|cutoff| observation.finished_at < cutoff)
        {
            return None;
        }
        if observation.finished_at
            > crate::time::now() + chrono::Duration::seconds(crate::observation::MAX_FUTURE_SKEW_SECONDS)
        {
            return None;
        }
        let mapping = self.mappings.get(&observation.probe_id)?.clone();
        if mapping.informational {
            return None;
        }
        let entity = observation.target_entity;
        let component = mapping.component;

        let key = (entity, observation.probe_id.clone(), observation.observer_entity);
        if let Some(stream) = self.streams.get(&key) {
            // A replay must not count toward hysteresis, and delayed spool
            // uploads must not overwrite a more recent verdict.
            if stream.last_observation == observation.id || stream.finished_at > observation.finished_at {
                return None;
            }
        }
        let previous = self
            .states
            .entry(entity)
            .or_insert_with(|| EntityState::unknown(entity))
            .component(component);
        let starting = if self
            .streams
            .values()
            .any(|s| s.entity == entity && s.component == component)
        {
            Health::Unknown
        } else {
            previous
        };
        let stream = self.streams.entry(key).or_insert_with(|| StateStream {
            entity,
            probe: observation.probe_id.clone(),
            observer: observation.observer_entity,
            component,
            last_observation: observation.id,
            finished_at: observation.finished_at,
            debouncer: Debouncer::starting_at(mapping.policy, starting),
            state: ComponentState::new(starting),
            expired: false,
        });
        stream.expired = false;
        stream.last_observation = observation.id;
        stream.finished_at = observation.finished_at;
        let health = match observation.status {
            ProbeStatus::Unsupported => {
                stream.debouncer.force(Health::Unknown);
                Health::Unknown
            }
            ProbeStatus::NotApplicable => {
                stream.debouncer.force(Health::NotApplicable);
                Health::NotApplicable
            }
            status => {
                stream.debouncer.observe(is_bad(status));
                cap(stream.debouncer.health(), status)
            }
        };
        if stream.state.health != health {
            stream.state.since = observation.finished_at;
        }
        stream.state.health = health;
        stream.state.evidence = vec![observation.id];
        stream.state.consecutive_failures = stream.debouncer.consecutive_failures();
        stream.state.consecutive_successes = stream.debouncer.consecutive_successes();
        self.recompute(entity, component)
    }

    fn recompute(&mut self, entity: EntityId, component: StateComponent) -> Option<StateTransition> {
        let contributors: Vec<_> = self
            .streams
            .values()
            .filter(|s| s.entity == entity && s.component == component && self.probe_enabled(entity, &s.probe))
            .collect();
        // A reassigned observer's old answer stops voting when other observers
        // still measure this same probe. A missing *probe* remains Unknown.
        let contributors: Vec<_> = contributors
            .iter()
            .filter(|s| {
                !s.expired
                    || !contributors
                        .iter()
                        .any(|other| other.probe == s.probe && !other.expired)
            })
            .copied()
            .collect();
        let worst = contributors
            .iter()
            .max_by_key(|s| (s.state.health.severity_rank(), s.probe.clone(), s.observer));
        let mut combined = worst.map_or_else(|| ComponentState::new(Health::NotApplicable), |s| s.state.clone());
        combined.evidence = contributors
            .iter()
            .flat_map(|s| s.state.evidence.iter().copied())
            .collect();
        combined.evidence.sort();
        combined.evidence.dedup();
        let state = self
            .states
            .entry(entity)
            .or_insert_with(|| EntityState::unknown(entity));
        let previous = state.component(component);
        let health = combined.health;
        if previous == health {
            combined.since = state
                .components
                .get(&component)
                .map(|c| c.since)
                .unwrap_or(combined.since);
        }
        let evidence = combined.evidence.clone();
        state.set_component(component, combined);
        (health != previous)
            .then(|| StateTransition::new(entity, Some(component), previous, health).with_evidence(evidence))
    }

    /// Expire health evidence without interpreting silence as recovery.
    pub fn expire(
        &mut self,
        at: Timestamp,
        horizon: impl Fn(&StateStream) -> chrono::Duration,
    ) -> Vec<StateTransition> {
        let mut changed = std::collections::BTreeSet::new();
        for stream in self.streams.values_mut() {
            if self.disabled.contains(&(stream.entity, stream.probe.clone())) {
                continue;
            }
            if at - stream.finished_at > horizon(stream) && stream.state.health != Health::NotApplicable {
                stream.expired = true;
                stream.state.health = Health::Unknown;
                stream.debouncer.force(Health::Unknown);
                changed.insert((stream.entity, stream.component));
            }
        }
        changed.into_iter().filter_map(|(e, c)| self.recompute(e, c)).collect()
    }

    /// Whether this source's current verdict has met the recovery threshold.
    pub fn confirms_health(&self, observation: &Observation) -> bool {
        if !self.probe_enabled(observation.target_entity, &observation.probe_id) {
            return false;
        }
        self.streams
            .get(&(
                observation.target_entity,
                observation.probe_id.clone(),
                observation.observer_entity,
            ))
            .map(|s| s.state.health == Health::Healthy && !s.expired)
            .unwrap_or_else(|| !self.knows(&observation.probe_id))
    }

    pub fn streams(&self) -> impl Iterator<Item = &StateStream> {
        self.streams.values()
    }

    pub fn seed_streams(&mut self, streams: impl IntoIterator<Item = StateStream>) {
        let mut components = std::collections::BTreeSet::new();
        for stream in streams {
            components.insert((stream.entity, stream.component));
            self.streams
                .insert((stream.entity, stream.probe.clone(), stream.observer), stream);
        }
        for (e, c) in components {
            self.recompute(e, c);
        }
    }

    /// Fold in many observations, in the order given.
    pub fn ingest_all<'a>(&mut self, observations: impl IntoIterator<Item = &'a Observation>) -> Vec<StateTransition> {
        observations.into_iter().filter_map(|o| self.ingest(o)).collect()
    }

    /// The state of one entity, if any observation has ever mentioned it.
    pub fn state(&self, entity: EntityId) -> Option<&EntityState> {
        self.states.get(&entity)
    }

    /// Every derived state, in entity order.
    pub fn states(&self) -> impl Iterator<Item = &EntityState> {
        self.states.values()
    }

    /// One entity's state, mutably, so a diagnosis can attach a classification.
    ///
    /// Classifications come from rules rather than probes, because only a rule
    /// has weighed enough evidence to justify one.
    pub fn state_mut(&mut self, entity: EntityId) -> Option<&mut EntityState> {
        self.states.get_mut(&entity)
    }

    /// Seed an entity's state, for rehydrating from the database.
    pub fn seed(&mut self, state: EntityState) {
        self.states.insert(state.entity, state);
    }
}

/// Whether a status should count against the entity in the debouncer.
fn is_bad(status: ProbeStatus) -> bool {
    !matches!(status, ProbeStatus::Ok)
}

/// The worst health a given status can justify.
fn cap(health: Health, status: ProbeStatus) -> Health {
    let ceiling = match status {
        ProbeStatus::Ok => return health,
        ProbeStatus::Degraded => Health::Degraded,
        _ => Health::Unavailable,
    };
    if health.severity_rank() > ceiling.severity_rank() {
        ceiling
    } else {
        health
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{EntityKey, EntityType};

    fn entity(name: &str) -> EntityId {
        EntityKey::new("lab", EntityType::Host, name).entity_id()
    }

    fn observation(probe: &str, target: EntityId, status: ProbeStatus) -> Observation {
        Observation::new(ProbeId::new(probe), target, status)
    }

    fn engine_with(probe: &str, mapping: ProbeMapping) -> StateEngine {
        let mut engine = StateEngine::new();
        engine.register(probe, mapping);
        engine
    }

    #[test]
    fn disabling_one_probe_does_not_hide_other_failures_in_its_component() {
        let mut engine = engine_with("gpu", ProbeMapping::immediate(StateComponent::Accelerator));
        engine.register("other", ProbeMapping::immediate(StateComponent::Accelerator));
        engine.ingest(&observation("gpu", entity("a"), ProbeStatus::Failed));
        engine.ingest(&observation("other", entity("a"), ProbeStatus::Failed));
        engine.set_probe_enabled(entity("a"), &ProbeId::new("gpu"), false);
        assert_eq!(
            engine
                .state(entity("a"))
                .unwrap()
                .component(StateComponent::Accelerator),
            Health::Unavailable
        );
        engine.ingest(&observation("other", entity("a"), ProbeStatus::Ok));
        assert_eq!(engine.state(entity("a")).unwrap().overall, Health::Healthy);
    }

    #[test]
    fn a_healthy_probe_cannot_hide_a_failed_probe_in_the_same_component() {
        let mut engine = engine_with("metrics", ProbeMapping::immediate(StateComponent::Host));
        engine.register("journal", ProbeMapping::immediate(StateComponent::Host));
        engine.ingest(&observation("metrics", entity("a"), ProbeStatus::Failed));
        for _ in 0..5 {
            engine.ingest(&observation("journal", entity("a"), ProbeStatus::Ok));
        }
        assert_eq!(engine.state(entity("a")).unwrap().overall, Health::Unavailable);
        engine.ingest(&observation("metrics", entity("a"), ProbeStatus::Ok));
        assert_eq!(engine.state(entity("a")).unwrap().overall, Health::Healthy);
    }

    #[test]
    fn healthy_observers_cannot_erase_a_failing_observers_path() {
        let mut engine = engine_with("network", ProbeMapping::new(StateComponent::Network));
        for _ in 0..3 {
            engine.ingest(&observation("network", entity("a"), ProbeStatus::Timeout).with_observer(entity("bad")));
            engine.ingest(&observation("network", entity("a"), ProbeStatus::Ok).with_observer(entity("good")));
        }
        assert_eq!(engine.state(entity("a")).unwrap().overall, Health::Unavailable);
    }

    #[test]
    fn unsupported_is_unknown_and_does_not_remove_a_known_failed_probe() {
        let mut engine = engine_with("storage", ProbeMapping::immediate(StateComponent::Storage));
        engine.ingest(&observation("storage", entity("a"), ProbeStatus::Failed));
        engine.ingest(&observation("storage", entity("a"), ProbeStatus::Unsupported));
        assert_eq!(engine.state(entity("a")).unwrap().overall, Health::Unknown);
    }

    #[test]
    fn applicability_changes_reset_the_hysteresis_history() {
        let mut engine = engine_with("gpu", ProbeMapping::new(StateComponent::Accelerator));
        for _ in 0..3 {
            engine.ingest(&observation("gpu", entity("a"), ProbeStatus::Failed));
        }
        engine.ingest(&observation("gpu", entity("a"), ProbeStatus::NotApplicable));
        engine.ingest(&observation("gpu", entity("a"), ProbeStatus::Ok));
        assert_eq!(
            engine
                .state(entity("a"))
                .unwrap()
                .component(StateComponent::Accelerator),
            Health::NotApplicable
        );
        engine.ingest(&observation("gpu", entity("a"), ProbeStatus::Ok));
        assert_eq!(engine.state(entity("a")).unwrap().overall, Health::Healthy);
    }

    #[test]
    fn unchanged_health_still_refreshes_its_evidence_and_counters() {
        let mut engine = engine_with("metrics", ProbeMapping::immediate(StateComponent::Host));
        engine.ingest(&observation("metrics", entity("a"), ProbeStatus::Ok));
        let since = engine.state(entity("a")).unwrap().components[&StateComponent::Host].since;
        let latest = observation("metrics", entity("a"), ProbeStatus::Ok);
        assert!(engine.ingest(&latest).is_none());
        let component = &engine.state(entity("a")).unwrap().components[&StateComponent::Host];
        assert_eq!(component.evidence, vec![latest.id]);
        assert_eq!(component.consecutive_successes, 2);
        assert_eq!(component.since, since);
    }

    #[test]
    fn an_unregistered_probe_changes_no_state() {
        // Guessing what an undeclared probe means would be worse than ignoring
        // it: the guess would be invisible and wrong.
        let mut engine = StateEngine::new();
        assert!(engine
            .ingest(&observation("mystery", entity("a"), ProbeStatus::Failed))
            .is_none());
        assert!(engine.state(entity("a")).is_none());
    }

    #[test]
    fn a_registered_probe_drives_its_declared_component() {
        let mut engine = engine_with("ssh.tcp", ProbeMapping::immediate(StateComponent::Ssh));
        let transition = engine
            .ingest(&observation("ssh.tcp", entity("a"), ProbeStatus::Ok))
            .expect("transition");

        assert_eq!(transition.component, Some(StateComponent::Ssh));
        assert_eq!(transition.from, Health::NotApplicable);
        assert_eq!(transition.to, Health::Healthy);
        assert_eq!(
            engine.state(entity("a")).unwrap().component(StateComponent::Ssh),
            Health::Healthy
        );
    }

    #[test]
    fn debouncing_holds_state_until_the_threshold_is_met() {
        let mut engine = engine_with("net.tcp", ProbeMapping::new(StateComponent::Network));
        for _ in 0..2 {
            engine.ingest(&observation("net.tcp", entity("a"), ProbeStatus::Ok));
        }
        assert_eq!(
            engine.state(entity("a")).unwrap().component(StateComponent::Network),
            Health::Healthy
        );

        assert!(
            engine
                .ingest(&observation("net.tcp", entity("a"), ProbeStatus::Failed))
                .is_none(),
            "one failure is not an outage"
        );
        let transition = engine
            .ingest(&observation("net.tcp", entity("a"), ProbeStatus::Failed))
            .expect("second failure warns");
        assert_eq!(transition.to, Health::Degraded);

        let transition = engine
            .ingest(&observation("net.tcp", entity("a"), ProbeStatus::Failed))
            .expect("third failure escalates");
        assert_eq!(transition.to, Health::Unavailable);
    }

    #[test]
    fn a_persistently_degraded_result_never_escalates_to_unavailable() {
        // Slow storage stays slow storage, however long it lasts.
        let mut engine = engine_with("nfs.latency", ProbeMapping::new(StateComponent::Storage));
        for _ in 0..10 {
            engine.ingest(&observation("nfs.latency", entity("a"), ProbeStatus::Degraded));
        }
        assert_eq!(
            engine.state(entity("a")).unwrap().component(StateComponent::Storage),
            Health::Degraded
        );
    }

    #[test]
    fn an_immediate_mapping_reacts_to_a_single_authoritative_report() {
        // A scheduler saying DRAIN is a statement, not a flaky measurement.
        let mut engine = engine_with("slurm.node", ProbeMapping::immediate(StateComponent::Scheduler));
        let transition = engine
            .ingest(&observation("slurm.node", entity("a"), ProbeStatus::Degraded))
            .expect("transition");
        assert_eq!(transition.to, Health::Degraded);
    }

    #[test]
    fn a_not_applicable_result_leaves_the_component_inapplicable_and_reports_no_change() {
        // An unrecorded component already reads as inapplicable, so learning
        // that a node has no GPUs is not a state change to report.
        let mut engine = engine_with("gpu.count", ProbeMapping::new(StateComponent::Accelerator));
        assert!(engine
            .ingest(&observation("gpu.count", entity("a"), ProbeStatus::NotApplicable))
            .is_none());

        let state = engine.state(entity("a")).expect("entity is known");
        assert_eq!(state.component(StateComponent::Accelerator), Health::NotApplicable);
        assert_eq!(
            state.overall,
            Health::Unknown,
            "an inapplicable component must not make the entity look healthy"
        );
    }

    #[test]
    fn a_component_that_stops_applying_transitions_out_of_its_old_health() {
        // GPUs removed from a node: the accelerator component must stop
        // reporting the stale verdict rather than freezing on it.
        let mut engine = engine_with("gpu.count", ProbeMapping::immediate(StateComponent::Accelerator));
        engine.ingest(&observation("gpu.count", entity("a"), ProbeStatus::Failed));
        assert_eq!(
            engine
                .state(entity("a"))
                .unwrap()
                .component(StateComponent::Accelerator),
            Health::Unavailable
        );

        let transition = engine
            .ingest(&observation("gpu.count", entity("a"), ProbeStatus::NotApplicable))
            .expect("transition out of unavailable");
        assert_eq!(transition.from, Health::Unavailable);
        assert_eq!(transition.to, Health::NotApplicable);
    }

    #[test]
    fn components_are_tracked_independently_per_entity() {
        let mut engine = StateEngine::new();
        engine.register("ssh.tcp", ProbeMapping::immediate(StateComponent::Ssh));
        engine.register("net.tcp", ProbeMapping::immediate(StateComponent::Network));

        engine.ingest(&observation("ssh.tcp", entity("a"), ProbeStatus::Failed));
        engine.ingest(&observation("net.tcp", entity("a"), ProbeStatus::Ok));
        engine.ingest(&observation("ssh.tcp", entity("b"), ProbeStatus::Ok));

        let a = engine.state(entity("a")).expect("a");
        assert_eq!(a.component(StateComponent::Ssh), Health::Unavailable);
        assert_eq!(a.component(StateComponent::Network), Health::Healthy);
        assert_eq!(a.overall, Health::Unavailable);

        assert_eq!(engine.state(entity("b")).unwrap().overall, Health::Healthy);
    }

    #[test]
    fn recovery_requires_the_configured_number_of_successes() {
        let mut engine = engine_with("net.tcp", ProbeMapping::new(StateComponent::Network));
        for _ in 0..3 {
            engine.ingest(&observation("net.tcp", entity("a"), ProbeStatus::Failed));
        }
        assert_eq!(
            engine.state(entity("a")).unwrap().component(StateComponent::Network),
            Health::Unavailable
        );

        assert!(engine
            .ingest(&observation("net.tcp", entity("a"), ProbeStatus::Ok))
            .is_none());
        let transition = engine
            .ingest(&observation("net.tcp", entity("a"), ProbeStatus::Ok))
            .expect("recovered");
        assert_eq!(transition.to, Health::Healthy);
    }

    #[test]
    fn transitions_carry_the_observation_that_caused_them() {
        let mut engine = engine_with("ssh.tcp", ProbeMapping::immediate(StateComponent::Ssh));
        let observation = observation("ssh.tcp", entity("a"), ProbeStatus::Failed);
        let transition = engine.ingest(&observation).expect("transition");
        assert_eq!(transition.evidence, vec![observation.id]);
    }

    #[test]
    fn seeded_state_is_resumed_rather_than_relearned() {
        // After a controller restart, a host that was already down must not
        // need three fresh failures to be considered down again.
        let mut engine = engine_with("net.tcp", ProbeMapping::new(StateComponent::Network));
        let mut state = EntityState::unknown(entity("a"));
        state.set_component(StateComponent::Network, ComponentState::new(Health::Unavailable));
        engine.seed(state);

        assert_eq!(
            engine.state(entity("a")).unwrap().component(StateComponent::Network),
            Health::Unavailable
        );
        assert!(engine
            .ingest(&observation("net.tcp", entity("a"), ProbeStatus::Ok))
            .is_none());
        let transition = engine
            .ingest(&observation("net.tcp", entity("a"), ProbeStatus::Ok))
            .expect("recovered");
        assert_eq!(transition.from, Health::Unavailable);
    }

    #[test]
    fn ingesting_a_batch_returns_only_the_actual_changes() {
        let mut engine = engine_with("ssh.tcp", ProbeMapping::immediate(StateComponent::Ssh));
        let observations = vec![
            observation("ssh.tcp", entity("a"), ProbeStatus::Ok),
            observation("ssh.tcp", entity("a"), ProbeStatus::Ok),
            observation("ssh.tcp", entity("a"), ProbeStatus::Failed),
        ];
        let transitions = engine.ingest_all(&observations);
        assert_eq!(transitions.len(), 2, "healthy -> healthy is not a change");
    }
}
