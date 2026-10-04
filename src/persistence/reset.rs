//! Reset current monitoring state without deleting observation history.

use serde::Serialize;

use super::{SqliteStore, StoreError};
use crate::time::{now, parse_rfc3339, to_rfc3339, Timestamp};

#[derive(Debug, Serialize)]
pub struct StateReset {
    pub reset_at: Timestamp,
    pub entities: u64,
    pub incidents: u64,
    pub pending_notifications: u64,
}

impl SqliteStore {
    /// The oldest measurement allowed to contribute to current state.
    pub async fn state_reset_at(&self, environment: &str) -> Result<Option<Timestamp>, StoreError> {
        let value: Option<String> = sqlx::query_scalar("SELECT reset_at FROM state_resets WHERE environment = ?")
            .bind(environment)
            .fetch_optional(self.pool())
            .await?;
        value
            .map(|value| {
                parse_rfc3339(&value).map_err(|error| StoreError::Decode {
                    kind: "state reset timestamp",
                    detail: error.to_string(),
                })
            })
            .transpose()
    }

    /// The controller must be stopped so it cannot write its old in-memory
    /// state back after this transaction. History and delivery records remain.
    pub async fn reset_state(&self, environment: &str) -> Result<StateReset, StoreError> {
        let mut tx = self.pool().begin().await?;
        let reset_at = now();
        sqlx::query("INSERT INTO state_resets(environment, reset_at) VALUES (?, ?) ON CONFLICT(environment) DO UPDATE SET reset_at = excluded.reset_at")
            .bind(environment).bind(to_rfc3339(reset_at)).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM entity_states WHERE entity_id IN (SELECT id FROM entities WHERE environment = ?)")
            .bind(environment)
            .execute(&mut *tx)
            .await?;
        let entities = sqlx::query(
            "DELETE FROM entity_overall_states WHERE entity_id IN (SELECT id FROM entities WHERE environment = ?)",
        )
        .bind(environment)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("DELETE FROM state_streams WHERE environment = ?")
            .bind(environment)
            .execute(&mut *tx)
            .await?;
        let incident_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM incidents WHERE environment = ? AND status IN ('open', 'recovering', 'acknowledged', 'suppressed')",
        )
        .bind(environment)
        .fetch_all(&mut *tx)
        .await?;
        for id in &incident_ids {
            sqlx::query("INSERT INTO incident_timeline(id, incident_id, occurred_at, kind, detail) VALUES (?, ?, ?, 'state_reset', ?)")
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(id)
                .bind(to_rfc3339(reset_at))
                .bind("監視状態を初期化したため、この障害の追跡を終了しました。")
                .execute(&mut *tx)
                .await?;
        }
        let incidents = sqlx::query("UPDATE incidents SET status = 'reset', ended_at = ?, updated_at = ? WHERE environment = ? AND status IN ('open', 'recovering', 'acknowledged', 'suppressed')")
            .bind(to_rfc3339(reset_at))
            .bind(to_rfc3339(reset_at))
            .bind(environment)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let pending_notifications = sqlx::query(
            "DELETE FROM notification_candidates WHERE incident_id IN (SELECT id FROM incidents WHERE environment = ?)",
        )
        .bind(environment)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(StateReset {
            reset_at,
            entities,
            incidents,
            pending_notifications,
        })
    }
}
