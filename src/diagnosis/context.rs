//! What a diagnosis rule is allowed to look at.
//!
//! Rules see derived state, the dependency graph, and the observations behind
//! them. They see nothing else — no network access, no commands, no clock of
//! their own — so a diagnosis is reproducible from stored data alone. That is
//! what makes an explanation checkable after the fact rather than something an
//! operator has to take on trust.

use std::collections::HashMap;

use crate::entity::{EntityId, EntityType, ManagedEntity};
use crate::inventory::Inventory;
use crate::observation::Observation;
use crate::probes::ProbeId;
use crate::state::{EntityState, Health, StateComponent};

/// The most recent observation per entity and probe.
#[derive(Debug, Default, Clone)]
pub struct ObservationIndex {
    latest: HashMap<(EntityId, ProbeId), Observation>,
}

impl ObservationIndex {
    /// An empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an observation, keeping the newest per entity and probe.
    pub fn insert(&mut self, observation: Observation) {
        let key = (observation.target_entity, observation.probe_id.clone());
        match self.latest.get(&key) {
            Some(existing) if existing.finished_at >= observation.finished_at => {}
            _ => {
                self.latest.insert(key, observation);
            }
        }
    }

    /// Build an index from observations.
    pub fn from_observations(observations: impl IntoIterator<Item = Observation>) -> Self {
        let mut index = Self::new();
        for observation in observations {
            index.insert(observation);
        }
        index
    }

    /// The latest observation of one probe against one entity.
    pub fn latest(&self, entity: EntityId, probe: &str) -> Option<&Observation> {
        self.latest.get(&(entity, ProbeId::new(probe)))
    }

    /// Every observation held for one entity.
    pub fn for_entity(&self, entity: EntityId) -> Vec<&Observation> {
        self.latest
            .iter()
            .filter(|((e, _), _)| *e == entity)
            .map(|(_, o)| o)
            .collect()
    }

    /// How many observations are indexed.
    pub fn len(&self) -> usize {
        self.latest.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.latest.is_empty()
    }
}

/// Everything a rule may consult.
pub struct DiagnosisContext<'a> {
    /// The environment being diagnosed.
    pub environment: &'a str,
    /// Entities and their dependency graph.
    pub inventory: &'a Inventory,
    /// Derived state, by entity.
    pub states: &'a HashMap<EntityId, EntityState>,
    /// The observations behind that state.
    pub observations: &'a ObservationIndex,
}

impl DiagnosisContext<'_> {
    /// An entity by id.
    pub fn entity(&self, id: EntityId) -> Option<&ManagedEntity> {
        self.inventory.get(id)
    }

    /// An entity's state.
    pub fn state(&self, id: EntityId) -> Option<&EntityState> {
        self.states.get(&id)
    }

    /// One component's health, `Unknown` if the entity has no state at all.
    pub fn component(&self, id: EntityId, component: StateComponent) -> Health {
        self.states
            .get(&id)
            .map(|s| s.component(component))
            .unwrap_or(Health::Unknown)
    }

    /// Whether a component is positively known to be healthy.
    ///
    /// `Unknown` and `NotApplicable` both answer *no*. A rule that concluded
    /// something from an absence of evidence would be guessing, and the guess
    /// would be invisible in the result.
    pub fn is_healthy(&self, id: EntityId, component: StateComponent) -> bool {
        self.component(id, component) == Health::Healthy
    }

    /// Whether every one of these components is positively healthy.
    pub fn all_healthy(&self, id: EntityId, components: &[StateComponent]) -> bool {
        components.iter().all(|c| self.is_healthy(id, *c))
    }

    /// Entities of one type, in a stable order.
    pub fn entities_of_type(&self, entity_type: EntityType) -> Vec<&ManagedEntity> {
        self.inventory
            .entities()
            .filter(|e| e.entity_type == entity_type)
            .filter(|e| e.lifecycle_state == crate::entity::LifecycleState::Active)
            .collect()
    }

    /// The latest observation of one probe against one entity.
    pub fn observation(&self, entity: EntityId, probe: &str) -> Option<&Observation> {
        self.observations.latest(entity, probe)
    }

    /// A boolean field from a probe's latest payload.
    pub fn payload_bool(&self, entity: EntityId, probe: &str, field: &str) -> Option<bool> {
        self.observation(entity, probe)?.payload.get(field)?.as_bool()
    }

    /// A string field from a probe's latest payload.
    pub fn payload_str(&self, entity: EntityId, probe: &str, field: &str) -> Option<&str> {
        self.observation(entity, probe)?.payload.get(field)?.as_str()
    }

    /// An integer field from a probe's latest payload.
    pub fn payload_u64(&self, entity: EntityId, probe: &str, field: &str) -> Option<u64> {
        self.observation(entity, probe)?.payload.get(field)?.as_u64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKey;
    use crate::observation::ProbeStatus;
    use crate::state::ComponentState;

    fn entity() -> EntityId {
        EntityKey::new("lab", EntityType::Host, "node-a").entity_id()
    }

    fn observation(probe: &str, status: ProbeStatus) -> Observation {
        Observation::new(ProbeId::new(probe), entity(), status)
    }

    #[test]
    fn the_index_keeps_the_newest_observation_per_probe() {
        let older = observation("ssh.service", ProbeStatus::Failed);
        let newer = observation("ssh.service", ProbeStatus::Ok).with_times(
            older.finished_at + chrono::Duration::seconds(10),
            older.finished_at + chrono::Duration::seconds(10),
        );

        let index = ObservationIndex::from_observations([older, newer.clone()]);
        assert_eq!(index.len(), 1);
        assert_eq!(index.latest(entity(), "ssh.service").expect("latest").id, newer.id);
    }

    #[test]
    fn an_older_observation_arriving_late_does_not_overwrite_a_newer_one() {
        // Spool replay delivers out of order; the newest reading must win.
        let newer = observation("ssh.service", ProbeStatus::Ok);
        let older = observation("ssh.service", ProbeStatus::Failed).with_times(
            newer.finished_at - chrono::Duration::seconds(60),
            newer.finished_at - chrono::Duration::seconds(60),
        );

        let index = ObservationIndex::from_observations([newer.clone(), older]);
        assert_eq!(index.latest(entity(), "ssh.service").expect("latest").id, newer.id);
    }

    #[test]
    fn different_probes_are_indexed_separately() {
        let index = ObservationIndex::from_observations([
            observation("ssh.service", ProbeStatus::Ok),
            observation("network.tcp", ProbeStatus::Failed),
        ]);
        assert_eq!(index.len(), 2);
        assert_eq!(index.for_entity(entity()).len(), 2);
    }

    #[test]
    fn an_unknown_component_is_not_treated_as_healthy() {
        // The distinction that stops a rule concluding something from silence.
        let inventory = Inventory::new();
        let states = HashMap::new();
        let observations = ObservationIndex::new();
        let context = DiagnosisContext {
            environment: "lab",
            inventory: &inventory,
            states: &states,
            observations: &observations,
        };

        assert_eq!(context.component(entity(), StateComponent::Ssh), Health::Unknown);
        assert!(!context.is_healthy(entity(), StateComponent::Ssh));
        assert!(!context.all_healthy(entity(), &[StateComponent::Ssh]));
    }

    #[test]
    fn a_not_applicable_component_is_not_treated_as_healthy_either() {
        let inventory = Inventory::new();
        let mut state = EntityState::unknown(entity());
        state.set_component(StateComponent::Accelerator, ComponentState::new(Health::NotApplicable));
        let states = HashMap::from([(entity(), state)]);
        let observations = ObservationIndex::new();

        let context = DiagnosisContext {
            environment: "lab",
            inventory: &inventory,
            states: &states,
            observations: &observations,
        };
        assert!(!context.is_healthy(entity(), StateComponent::Accelerator));
    }

    #[test]
    fn payload_fields_are_reachable_by_type() {
        let inventory = Inventory::new();
        let states = HashMap::new();
        let observations = ObservationIndex::from_observations([observation("slurm.node", ProbeStatus::Degraded)
            .with_payload(serde_json::json!({"drained": true, "state": "IDLE+DRAIN", "cpu_total": 64}))]);

        let context = DiagnosisContext {
            environment: "lab",
            inventory: &inventory,
            states: &states,
            observations: &observations,
        };

        assert_eq!(context.payload_bool(entity(), "slurm.node", "drained"), Some(true));
        assert_eq!(context.payload_str(entity(), "slurm.node", "state"), Some("IDLE+DRAIN"));
        assert_eq!(context.payload_u64(entity(), "slurm.node", "cpu_total"), Some(64));
        assert_eq!(context.payload_bool(entity(), "slurm.node", "missing"), None);
        assert_eq!(context.payload_bool(entity(), "never.ran", "drained"), None);
    }
}
