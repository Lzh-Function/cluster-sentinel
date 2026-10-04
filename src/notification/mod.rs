//! Notifications.
//!
//! The hard part of alerting is not sending messages; it is **not** sending
//! them. A monitoring system that re-notifies every polling interval trains its
//! operators to filter it out, and then the one alert that mattered is filtered
//! out too.
//!
//! So notification here is driven by *change*, never by state
//! (IMPLEMENTATION.md §77). An incident that is still open and still says the
//! same thing produces nothing at all.
//!
//! Deduplication is separate from that and belongs to a different problem: the
//! controller and a fallback notifier may both notice the same incident, and
//! the operator should hear about it once (SPEC.md §110, §111).

mod dedup;
mod maintenance;
mod provider;

pub use dedup::{Deduplicator, NotificationRecord};
pub use maintenance::{MaintenanceWindow, MaintenanceWindows};
pub use provider::{NotificationProvider, ProviderError, WebhookProvider};

use serde::{Deserialize, Serialize};

use crate::incident::{Incident, IncidentUpdate, Severity};
use crate::time::Timestamp;

/// Why a notification is being sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// A new incident was opened.
    Opened,
    /// An existing incident became more severe.
    Escalated,
    /// The diagnosis changed in a way worth reporting.
    DiagnosisChanged,
    /// The cause is gone but dependents have not recovered.
    Recovering,
    /// Everything involved has recovered.
    Resolved,
}

impl Trigger {
    /// Stable string form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Trigger::Opened => "opened",
            Trigger::Escalated => "escalated",
            Trigger::DiagnosisChanged => "diagnosis_changed",
            Trigger::Recovering => "recovering",
            Trigger::Resolved => "resolved",
        }
    }

    /// Whether this trigger is good news.
    pub fn is_recovery(&self) -> bool {
        matches!(self, Trigger::Recovering | Trigger::Resolved)
    }
}

/// A message about one incident.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    /// The incident this concerns.
    pub incident_id: String,
    /// The incident's fingerprint, which is also the deduplication scope.
    pub fingerprint: String,
    /// Why this is being sent.
    pub trigger: Trigger,
    /// Severity at the time of sending.
    pub severity: Severity,
    /// One-line summary.
    pub title: String,
    /// The detail an operator needs to decide whether to get up.
    pub body: String,
    /// Read-only commands worth running.
    pub recommended_actions: Vec<String>,
    /// When this was produced.
    pub created_at: Timestamp,
    /// Stable lifecycle event identity, so retries cannot become new news.
    #[serde(default)]
    pub event_id: Option<String>,
}

impl Notification {
    /// The key that decides whether this has already been sent.
    ///
    /// Scoped to the incident *and the trigger*, so a resolution is still
    /// delivered after the opening was: they are different news.
    pub fn deduplication_key(&self) -> String {
        match &self.event_id {
            Some(event) => format!(
                "{}:event:{}:{}:{}",
                self.fingerprint,
                self.incident_id,
                event,
                self.trigger.as_str()
            ),
            None => format!("{}:{}", self.fingerprint, self.trigger.as_str()),
        }
    }

    /// Build a notification for an incident.
    pub fn for_incident(incident: &Incident, trigger: Trigger) -> Self {
        let heading = heading_for(trigger, incident.severity);
        let summary = if trigger.is_recovery() {
            incident
                .primary_diagnosis()
                .map(|d| d.diagnosis_type.label().to_string())
                .unwrap_or_else(|| "障害".to_string())
        } else {
            summary_of(incident)
        };
        let title = format!("{heading} / {summary}");
        let mut body = match trigger {
            Trigger::Recovering => "原因と考えられる対象の復旧を確認しました。影響を受けた対象すべての復旧は、まだ確認できていません。\n\n発生時の診断\n".to_string(),
            Trigger::Resolved => "原因と考えられる対象と、影響を受けた対象すべての復旧を確認しました。\n\n発生時の診断\n".to_string(),
            Trigger::DiagnosisChanged => "観測結果の更新により、障害の診断が変わりました。最新の対象と確認手順を確認してください。\n\n".to_string(),
            _ => String::new(),
        };
        for diagnosis in &incident.diagnoses {
            let confidence = match diagnosis.confidence {
                crate::diagnosis::Confidence::Low => "低",
                crate::diagnosis::Confidence::Medium => "中",
                crate::diagnosis::Confidence::High => "高",
                crate::diagnosis::Confidence::Confirmed => "直接の証拠で確認済み",
            };
            body.push_str(&format!(
                "{}\n診断の確度　{confidence}\n\n{}\n\n",
                diagnosis.diagnosis_type.label(),
                diagnosis.summary
            ));
        }
        let status = match incident.status {
            crate::incident::IncidentStatus::Open => "未解決",
            crate::incident::IncidentStatus::Acknowledged => "確認済み・対応中",
            crate::incident::IncidentStatus::Recovering => "復旧途中",
            crate::incident::IncidentStatus::Resolved => "復旧確認済み",
            crate::incident::IncidentStatus::Reset => "初期化で終了",
            crate::incident::IncidentStatus::Suppressed => "通知を抑止中",
        };
        body.push_str(&format!(
            "障害ID　{}\n状態　{status}\n根拠となる観測　{}件\n",
            incident.id,
            incident.evidence.len()
        ));
        use crate::diagnosis::investigation as guide;
        let mut actions = vec![guide::step(
            "Sentinel controller",
            if trigger == Trigger::Resolved {
                "障害の履歴と復旧時刻を確認してください"
            } else {
                "原因の候補、影響を受けた対象、根拠となる観測と履歴を確認してください"
            },
            &guide::sentinel(&format!("incident show {}", incident.id)),
        )];
        if trigger != Trigger::Resolved {
            let mut seen = std::collections::HashSet::new();
            for action in incident.diagnoses.iter().flat_map(|d| &d.recommended_actions) {
                if seen.insert(action) {
                    actions.push(action.clone());
                }
            }
        }

        Self {
            incident_id: incident.id.to_string(),
            fingerprint: incident.fingerprint.clone(),
            trigger,
            severity: incident.severity,
            title,
            body,
            recommended_actions: actions,
            created_at: event_for(incident, trigger)
                .map(|e| e.at)
                .unwrap_or(incident.started_at),
            event_id: Some(event_identity(incident, trigger)),
        }
    }
}

pub(crate) fn heading_for(trigger: Trigger, severity: Severity) -> String {
    let level = match severity {
        Severity::Critical => "重大",
        Severity::Warning => "警告",
        Severity::Info => "情報",
    };
    match trigger {
        Trigger::Opened => format!("障害発生（{level}）"),
        Trigger::Escalated => format!("重大度上昇（{level}）"),
        Trigger::DiagnosisChanged => format!("診断更新（{level}）"),
        Trigger::Recovering => "復旧途中".into(),
        Trigger::Resolved => "復旧確認".into(),
    }
}

fn summary_of(incident: &Incident) -> String {
    incident
        .primary_diagnosis()
        .map(|d| d.summary.clone())
        .unwrap_or_else(|| format!("障害 {}", incident.fingerprint))
}

/// Turn a reconciliation into the notifications worth sending.
///
/// This produces **candidates**, not messages. Whether a candidate is news is
/// the deduplicator's judgement, because that is the only part that knows what
/// has already been said -- and it knows it durably, across restarts.
///
/// The distinction matters because the alternative was silently lossy. When
/// this function emitted only what changed in the current pass, an incident
/// had exactly one chance to be announced, in the fifteen seconds it was
/// opened. Miss that -- notifications not configured yet, a webhook returning
/// 500, a controller restarted in that tick -- and the incident stayed silent
/// for the rest of its life while `sentinel incident list` showed it open. A
/// monitoring system that has noticed a fault and says nothing is worse than
/// one that never noticed.
pub fn notifications_for(update: &IncidentUpdate) -> Vec<Notification> {
    let mut by_key = std::collections::BTreeMap::new();
    for incident in update.all() {
        for notification in notifications_for_incident(incident) {
            by_key.insert(notification.deduplication_key(), notification);
        }
    }
    let mut notifications: Vec<_> = by_key.into_values().collect();
    notifications.sort_by_key(|n| (n.created_at, n.trigger));
    notifications
}

fn event_for(incident: &Incident, trigger: Trigger) -> Option<&crate::incident::TimelineEvent> {
    let kinds: &[&str] = match trigger {
        Trigger::Opened => &["opened", "reopened", "recovery_failed"],
        Trigger::Escalated => &["severity_escalated"],
        Trigger::DiagnosisChanged => &["diagnosis_changed"],
        Trigger::Recovering => &["recovering"],
        Trigger::Resolved => &["resolved"],
    };
    incident
        .timeline
        .iter()
        .rev()
        .find(|e| kinds.contains(&e.kind.as_str()))
}

fn event_identity(incident: &Incident, trigger: Trigger) -> String {
    let event = event_for(incident, trigger);
    let text = event
        .map(|e| format!("{}:{}:{}", crate::time::to_rfc3339(e.at), e.kind, e.detail))
        .unwrap_or_else(|| crate::time::to_rfc3339(incident.started_at));
    uuid::Uuid::new_v5(&incident.id, text.as_bytes()).to_string()
}

/// Current lifecycle candidates. Stored atomically with the incident, then
/// offered until each provider records delivery. Later lifecycle states replace
/// obsolete messages, so a recovery cannot arrive after a returning outage.
pub fn notifications_for_incident(incident: &Incident) -> Vec<Notification> {
    use crate::incident::IncidentStatus;
    let mut notifications = Vec::new();
    let mut push = |trigger| {
        let mut n = Notification::for_incident(incident, trigger);
        n.created_at = event_for(incident, trigger)
            .map(|e| e.at)
            .unwrap_or(incident.started_at);
        notifications.push(n);
    };
    match incident.status {
        IncidentStatus::Resolved => push(Trigger::Resolved),
        IncidentStatus::Recovering => push(Trigger::Recovering),
        IncidentStatus::Suppressed | IncidentStatus::Reset => {}
        IncidentStatus::Open | IncidentStatus::Acknowledged => {
            push(Trigger::Opened);
            let lifecycle = incident
                .timeline
                .iter()
                .rposition(|e| matches!(e.kind.as_str(), "opened" | "reopened" | "recovery_failed"));
            for (kind, trigger) in [
                ("severity_escalated", Trigger::Escalated),
                ("diagnosis_changed", Trigger::DiagnosisChanged),
            ] {
                if incident
                    .timeline
                    .iter()
                    .enumerate()
                    .any(|(index, event)| event.kind == kind && lifecycle.is_none_or(|start| index >= start))
                {
                    push(trigger);
                }
            }
        }
    }
    notifications
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnosis::{kind, Confidence, Diagnosis};
    use crate::incident::{IncidentStatus, TimelineEvent};
    use crate::observation::ObservationId;

    fn incident(severity: Severity) -> Incident {
        let mut incident = Incident::open("cause:fs1", severity);
        incident.add_diagnosis(
            Diagnosis::new(kind::NFS_SERVICE_FAILURE, "storage.service_failure", Confidence::High)
                .with_summary("fs1 is up but the export port is not answering")
                .with_evidence([ObservationId::new()])
                .recommending(vec!["systemctl status nfs-server".into()]),
        );
        incident
    }

    #[test]
    fn a_new_incident_produces_one_notification() {
        let update = IncidentUpdate {
            opened: vec![incident(Severity::Critical)],
            ..Default::default()
        };
        let notifications = notifications_for(&update);

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].trigger, Trigger::Opened);
        assert!(
            notifications[0].title.starts_with("障害発生（重大）"),
            "{}",
            notifications[0].title
        );
        assert!(
            notifications[0].body.contains("export port"),
            "{}",
            notifications[0].body
        );
    }

    #[test]
    fn an_unchanged_incident_offers_only_its_opening_and_only_once() {
        // Re-notifying every polling interval is how a monitoring system
        // teaches people to ignore it, so this is still the property that
        // matters -- but it is now enforced one layer down. Every pass offers
        // the opening of a still-open incident; the deduplicator sends it once
        // and never again. That is what lets an opening whose delivery failed
        // be delivered later instead of being lost with the tick it happened
        // in.
        let update = IncidentUpdate {
            updated: vec![incident(Severity::Critical)],
            ..Default::default()
        };
        let notifications = notifications_for(&update);

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].trigger, Trigger::Opened);

        let mut deduplicator = Deduplicator::new();
        assert!(deduplicator.should_send(&notifications[0], "webhook"));
        deduplicator.record(&notifications[0], "webhook");

        // Every subsequent pass, for as long as the incident lasts.
        for _ in 0..10 {
            let again = notifications_for(&update);
            assert!(!deduplicator.should_send(&again[0], "webhook"));
        }
    }

    #[test]
    fn a_resolved_incident_offers_no_opening() {
        // The opening candidate is scoped to incidents that are still active,
        // so a resolution is not accompanied by a stale announcement of it.
        let mut resolved = incident(Severity::Critical);
        resolved.resolve();

        let update = IncidentUpdate {
            updated: vec![resolved],
            ..Default::default()
        };
        assert!(notifications_for(&update)
            .iter()
            .all(|n| n.trigger == Trigger::Resolved));
    }

    #[test]
    fn an_escalation_is_news() {
        let mut incident = incident(Severity::Warning);
        incident.escalate(Severity::Critical);

        let update = IncidentUpdate {
            updated: vec![incident],
            ..Default::default()
        };
        let notifications = notifications_for(&update);
        let escalation = notifications
            .iter()
            .find(|n| n.trigger == Trigger::Escalated)
            .expect("an escalation is news");

        assert!(escalation.title.contains("重大度上昇"), "{}", escalation.title);
    }

    #[test]
    fn a_changed_diagnosis_is_news() {
        let mut incident = incident(Severity::Warning);
        incident.timeline.push(TimelineEvent::new(
            "diagnosis_changed",
            "NFS_SERVICE_FAILURE -> SHARED_STORAGE_FAILURE",
        ));

        let update = IncidentUpdate {
            updated: vec![incident],
            ..Default::default()
        };
        assert!(notifications_for(&update)
            .iter()
            .any(|n| n.trigger == Trigger::DiagnosisChanged));
    }

    #[test]
    fn a_reopened_incident_is_news() {
        let mut incident = incident(Severity::Warning);
        incident
            .timeline
            .push(TimelineEvent::new("reopened", "the fault returned"));

        let update = IncidentUpdate {
            updated: vec![incident],
            ..Default::default()
        };
        assert_eq!(notifications_for(&update)[0].trigger, Trigger::Opened);
    }

    #[test]
    fn recovery_and_resolution_are_both_reported() {
        // An operator who was told about the fault is owed the ending.
        let mut recovering = incident(Severity::Critical);
        recovering.status = IncidentStatus::Recovering;
        let mut resolved = incident(Severity::Critical);
        resolved.resolve();

        let update = IncidentUpdate {
            recovering: vec![recovering],
            resolved: vec![resolved],
            ..Default::default()
        };
        let triggers: Vec<_> = notifications_for(&update).into_iter().map(|n| n.trigger).collect();

        assert!(triggers.contains(&Trigger::Recovering));
        assert!(triggers.contains(&Trigger::Resolved));
    }

    #[test]
    fn a_resolution_notification_reads_as_good_news() {
        let mut resolved = incident(Severity::Critical);
        resolved.resolve();

        let notification = Notification::for_incident(&resolved, Trigger::Resolved);
        assert!(notification.title.starts_with("復旧確認"), "{}", notification.title);
        assert!(notification.trigger.is_recovery());
    }

    #[test]
    fn the_deduplication_key_separates_opening_from_resolution() {
        // Otherwise the resolution would be suppressed as a duplicate of the
        // alert, and the operator would never learn it was over.
        let incident = incident(Severity::Critical);
        let opened = Notification::for_incident(&incident, Trigger::Opened);
        let resolved = Notification::for_incident(&incident, Trigger::Resolved);

        assert_ne!(opened.deduplication_key(), resolved.deduplication_key());
        assert!(opened.deduplication_key().starts_with("cause:fs1"));
    }

    #[test]
    fn the_same_event_from_two_notifiers_shares_a_key() {
        // SPEC.md §111: the controller and a fallback notifier must not both
        // wake the same person.
        let incident = incident(Severity::Critical);
        let from_controller = Notification::for_incident(&incident, Trigger::Opened);
        let from_fallback = Notification::for_incident(&incident, Trigger::Opened);

        assert_eq!(from_controller.deduplication_key(), from_fallback.deduplication_key());
    }

    #[test]
    fn a_notification_carries_the_read_only_actions() {
        let notification = Notification::for_incident(&incident(Severity::Critical), Trigger::Opened);
        assert!(notification.recommended_actions[0].contains("sudo -u sentinel sentinel incident show"));
        assert!(notification
            .recommended_actions
            .iter()
            .any(|a| a == "systemctl status nfs-server"));
    }

    #[test]
    fn common_instructions_are_deduplicated_across_diagnoses() {
        let mut incident = incident(Severity::Critical);
        incident.add_diagnosis(
            Diagnosis::new(kind::SHARED_STORAGE_FAILURE, "storage.shared", Confidence::Medium).recommending(vec![
                "systemctl status nfs-server".into(),
                "共有設定を確認してください。".into(),
            ]),
        );
        let notification = Notification::for_incident(&incident, Trigger::Opened);
        assert_eq!(
            notification
                .recommended_actions
                .iter()
                .filter(|a| *a == "systemctl status nfs-server")
                .count(),
            1
        );
        assert!(notification.recommended_actions[0].contains(&incident.id.to_string()));
    }

    #[test]
    fn resolved_notifications_request_history_review_instead_of_fault_investigation() {
        let mut incident = incident(Severity::Critical);
        incident.resolve();
        let notification = Notification::for_incident(&incident, Trigger::Resolved);
        assert_eq!(notification.recommended_actions.len(), 1);
        assert!(notification.recommended_actions[0].contains("復旧時刻"));
        assert!(notification.body.contains("発生時の診断"));
        assert!(!notification.title.contains("not answering"));
    }

    #[test]
    fn an_empty_update_produces_nothing() {
        assert!(notifications_for(&IncidentUpdate::default()).is_empty());
    }
}
