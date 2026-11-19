//! Continuous agent runtimes (24/7 loop). The runtime asks the control plane to create
//! the next execution whenever the previous one reaches a terminal state.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{Store, StoreError, StoreResult};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntime {
    pub id: Uuid,
    pub org_id: Uuid,
    pub agent_id: Uuid,
    pub status: String,
    pub task: String,
    pub harness_kind: String,
    pub last_execution_id: Option<Uuid>,
    pub last_error: Option<String>,
    pub next_run_at: DateTime<Utc>,
    pub interval_seconds: i32,
    /// Failed executions in a row; reset when an execution succeeds.
    pub consecutive_failures: i32,
    /// Last failed execution already counted and backed off from.
    pub failed_execution_id: Option<Uuid>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}

impl Store {
    pub async fn create_agent_runtime(
        &self,
        org_id: Uuid,
        agent_id: Uuid,
        task: &str,
        harness_kind: &str,
        interval_seconds: i32,
        created_by: Option<Uuid>,
    ) -> StoreResult<AgentRuntime> {
        sqlx::query_as::<_, AgentRuntime>(
            r#"
            INSERT INTO agent_runtimes (id, org_id, agent_id, task, harness_kind, interval_seconds, created_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(Uuid::now_v7())
        .bind(org_id)
        .bind(agent_id)
        .bind(task)
        .bind(harness_kind)
        .bind(interval_seconds)
        .bind(created_by)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn get_agent_runtime(
        &self,
        org_id: Uuid,
        agent_id: Uuid,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            "SELECT * FROM agent_runtimes WHERE org_id = $1 AND agent_id = $2 AND status IN ('running','stopping') ORDER BY created_at DESC LIMIT 1",
        )
        .bind(org_id)
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_running_agent_runtimes(&self) -> StoreResult<Vec<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            "SELECT * FROM agent_runtimes WHERE status IN ('running','stopping') ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// Atomically claim the next tick for a runtime. Only one API replica can create the
    /// next execution; the short lease is released when the execution row is written.
    pub async fn heartbeat_agent_runtime(&self, runtime_id: Uuid) -> StoreResult<()> {
        sqlx::query(
            "UPDATE agent_runtimes SET last_heartbeat_at = now(), updated_at = now() WHERE id = $1 AND status IN ('running','stopping')",
        )
        .bind(runtime_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn claim_agent_runtime_tick(
        &self,
        runtime_id: Uuid,
        lease_until: DateTime<Utc>,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET updated_at = now(), next_run_at = $2
            WHERE id = $1 AND status = 'running' AND next_run_at <= now()
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(lease_until)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn update_agent_runtime(
        &self,
        runtime_id: Uuid,
        status: &str,
        last_execution_id: Option<Uuid>,
        last_error: Option<&str>,
        next_run_at: DateTime<Utc>,
    ) -> StoreResult<AgentRuntime> {
        sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = $2,
                last_execution_id = COALESCE($3, last_execution_id),
                last_error = $4,
                next_run_at = $5,
                updated_at = now(),
                stopped_at = CASE WHEN $2 = 'stopped' THEN now() ELSE stopped_at END
            WHERE id = $1
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(status)
        .bind(last_execution_id)
        .bind(last_error)
        .bind(next_run_at)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound("agent runtime".into()))
    }

    /// Record a failed execution: bump the counter, remember which execution it was and
    /// schedule the next attempt (or stop the runtime) in one statement.
    pub async fn record_agent_runtime_failure(
        &self,
        runtime_id: Uuid,
        status: &str,
        failed_execution_id: Uuid,
        consecutive_failures: i32,
        last_error: &str,
        next_run_at: DateTime<Utc>,
    ) -> StoreResult<AgentRuntime> {
        sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = $2,
                failed_execution_id = $3,
                consecutive_failures = $4,
                last_error = $5,
                next_run_at = $6,
                updated_at = now(),
                stopped_at = CASE WHEN $2 = 'stopped' THEN now() ELSE stopped_at END
            WHERE id = $1
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(status)
        .bind(failed_execution_id)
        .bind(consecutive_failures)
        .bind(last_error)
        .bind(next_run_at)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound("agent runtime".into()))
    }

    /// Record a newly created execution. `reset_failures` clears the failure counter and
    /// is set when the previous execution did not fail.
    pub async fn record_agent_runtime_launch(
        &self,
        runtime_id: Uuid,
        execution_id: Uuid,
        reset_failures: bool,
        next_run_at: DateTime<Utc>,
    ) -> StoreResult<AgentRuntime> {
        sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = 'running',
                last_execution_id = $2,
                last_error = NULL,
                consecutive_failures = CASE WHEN $3 THEN 0 ELSE consecutive_failures END,
                failed_execution_id = CASE WHEN $3 THEN NULL ELSE failed_execution_id END,
                next_run_at = $4,
                updated_at = now()
            WHERE id = $1
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(execution_id)
        .bind(reset_failures)
        .bind(next_run_at)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound("agent runtime".into()))
    }
}
