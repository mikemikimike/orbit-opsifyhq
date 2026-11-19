use chrono::Utc;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use orbit_domain::{new_id, TriggerSpec};

use crate::models::Trigger;
use crate::{Store, StoreError, StoreResult};

/// Replace the managed triggers for an agent with the ones declared in its spec.
///
/// Called inside the agent creation/version transaction. Manual executions are not
/// triggers; other kinds are fully derived from the spec. Trigger firing history
/// (`last_fired_at`) resets when a configuration version changes (documented).
pub(crate) async fn sync_agent_triggers(
    tx: &mut Transaction<'_, Postgres>,
    org_id: Uuid,
    agent_id: Uuid,
    triggers: &[TriggerSpec],
) -> StoreResult<()> {
    sqlx::query("DELETE FROM triggers WHERE org_id = $1 AND agent_id = $2 AND kind <> 'manual'")
        .bind(org_id)
        .bind(agent_id)
        .execute(&mut **tx)
        .await?;

    for spec in triggers {
        let (kind, config, next_run_at) = match spec {
            TriggerSpec::Schedule {
                cron,
                timezone,
                task,
            } => {
                let next = cron
                    .parse::<croner::Cron>()
                    .ok()
                    .and_then(|parsed| parsed.find_next_occurrence(&Utc::now(), false).ok());
                (
                    "schedule",
                    // The task travels in the trigger config, where the scheduler reads it.
                    match task
                        .as_deref()
                        .map(str::trim)
                        .filter(|task| !task.is_empty())
                    {
                        Some(task) => json!({ "cron": cron, "timezone": timezone, "task": task }),
                        None => json!({ "cron": cron, "timezone": timezone }),
                    },
                    next,
                )
            }
            TriggerSpec::Webhook { secret_ref } => {
                ("webhook", json!({ "secretRef": secret_ref }), None)
            }
            TriggerSpec::Event { subject } => ("event", json!({ "subject": subject }), None),
            TriggerSpec::Manual => continue,
        };

        sqlx::query(
            r#"
            INSERT INTO triggers (id, org_id, agent_id, kind, enabled, config, next_run_at)
            VALUES ($1, $2, $3, $4, true, $5, $6)
            "#,
        )
        .bind(new_id())
        .bind(org_id)
        .bind(agent_id)
        .bind(kind)
        .bind(config)
        .bind(next_run_at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

impl Store {
    pub async fn list_triggers(
        &self,
        org_id: Uuid,
        agent_id: Option<Uuid>,
        access: Option<&crate::access::ResourceAccess>,
    ) -> StoreResult<Vec<Trigger>> {
        let (privileged, user_id, team_ids, isolation) = crate::access::binds(access);
        Ok(sqlx::query_as::<_, Trigger>(
            r#"
            SELECT * FROM triggers
            WHERE org_id = $1 AND ($2::uuid IS NULL OR agent_id = $2)
              AND ($3::bool OR owner_user_id = $4
                   OR CASE WHEN $6::bool AND owner_team_id IS NOT NULL
                        THEN owner_team_id = ANY($5)
                        ELSE visibility = 'org'
                             OR (visibility = 'team' AND owner_team_id = ANY($5)) END)
            ORDER BY created_at DESC
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(privileged)
        .bind(user_id)
        .bind(&team_ids)
        .bind(isolation)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_trigger(&self, org_id: Uuid, trigger_id: Uuid) -> StoreResult<Trigger> {
        sqlx::query_as::<_, Trigger>("SELECT * FROM triggers WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(trigger_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::NotFound("trigger".into()))
    }

    /// Add a trigger outside of an agent version (e.g. an operator-managed webhook).
    pub async fn create_trigger(
        &self,
        org_id: Uuid,
        agent_id: Uuid,
        kind: &str,
        harness_kind: Option<&str>,
        config: serde_json::Value,
        enabled: bool,
    ) -> StoreResult<Trigger> {
        // Ensure the agent belongs to the org before creating the trigger.
        self.get_agent(org_id, agent_id).await?;
        if !matches!(kind, "schedule" | "webhook" | "event" | "manual") {
            return Err(StoreError::Invalid(format!("unknown trigger kind: {kind}")));
        }
        let next_run_at = if kind == "schedule" {
            config
                .get("cron")
                .and_then(|c| c.as_str())
                .and_then(|c| c.parse::<croner::Cron>().ok())
                .and_then(|parsed| parsed.find_next_occurrence(&Utc::now(), false).ok())
        } else {
            None
        };
        Ok(sqlx::query_as::<_, Trigger>(
            r#"
            INSERT INTO triggers (id, org_id, agent_id, kind, harness_kind, enabled, config, next_run_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(new_id())
        .bind(org_id)
        .bind(agent_id)
        .bind(kind)
        .bind(harness_kind)
        .bind(enabled)
        .bind(config)
        .bind(next_run_at)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn set_trigger_enabled(
        &self,
        org_id: Uuid,
        trigger_id: Uuid,
        enabled: bool,
    ) -> StoreResult<Trigger> {
        sqlx::query_as::<_, Trigger>(
            "UPDATE triggers SET enabled = $3, updated_at = now() WHERE org_id = $1 AND id = $2 RETURNING *",
        )
        .bind(org_id)
        .bind(trigger_id)
        .bind(enabled)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound("trigger".into()))
    }

    pub async fn delete_trigger(&self, org_id: Uuid, trigger_id: Uuid) -> StoreResult<()> {
        let result = sqlx::query("DELETE FROM triggers WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(trigger_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound("trigger".into()));
        }
        Ok(())
    }

    /// Due schedule triggers, claimed by updating `next_run_at`.
    pub async fn claim_due_triggers(&self, limit: i64) -> StoreResult<Vec<Trigger>> {
        let rows = sqlx::query_as::<_, Trigger>(
            r#"
            WITH due AS (
                SELECT id FROM triggers
                WHERE enabled AND kind = 'schedule' AND next_run_at IS NOT NULL AND next_run_at <= now()
                ORDER BY next_run_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            UPDATE triggers t
            SET next_run_at = NULL, last_fired_at = now(), updated_at = now()
            FROM due
            WHERE t.id = due.id
            RETURNING t.*
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Recompute the next occurrence for a schedule trigger after firing.
    pub async fn reschedule_trigger(
        &self,
        trigger_id: Uuid,
        next_run_at: Option<chrono::DateTime<Utc>>,
    ) -> StoreResult<()> {
        sqlx::query("UPDATE triggers SET next_run_at = $2, updated_at = now() WHERE id = $1")
            .bind(trigger_id)
            .bind(next_run_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

impl Store {
    /// Record a successful trigger fire.
    pub async fn record_trigger_success(&self, trigger_id: Uuid) -> StoreResult<()> {
        sqlx::query(
            "UPDATE triggers SET consecutive_failures = 0, updated_at = now() WHERE id = $1",
        )
        .bind(trigger_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record a failed trigger fire. Auto-disables after max_failures consecutive failures.
    pub async fn record_trigger_failure(&self, trigger_id: Uuid) -> StoreResult<bool> {
        let result = sqlx::query(
            r#"
            UPDATE triggers
            SET consecutive_failures = consecutive_failures + 1,
                enabled = CASE WHEN consecutive_failures + 1 >= max_failures THEN false ELSE enabled END,
                updated_at = now()
            WHERE id = $1
            RETURNING enabled
            "#,
        )
        .bind(trigger_id)
        .fetch_optional(&self.pool)
        .await?;
        match result {
            Some(row) => {
                let enabled: bool = sqlx::Row::get(&row, "enabled");
                Ok(!enabled) // true if disabled
            }
            None => Ok(false),
        }
    }
}
