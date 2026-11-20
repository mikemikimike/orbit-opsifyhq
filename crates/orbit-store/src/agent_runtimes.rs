//! Continuous agent runtimes (24/7 loop). The runtime asks the control plane to create
//! the next execution whenever the previous one reaches a terminal state.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{Store, StoreResult};

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

    // ---- Status transitions -------------------------------------------------------
    //
    // `status` is only ever changed by the explicit, conditional transitions below, so a
    // concurrent writer (an operator pressing Stop, a tick finishing late) can never
    // revive a runtime:
    //
    //   running  -> stopping   request_agent_runtime_stop   (WHERE status IN running|stopping)
    //   stopping -> stopped    finish_agent_runtime_stop    (WHERE status = 'stopping')
    //   running|stopping -> stopped
    //                          halt_agent_runtime           (agent gone/inactive)
    //   running  -> stopped    halt_running_agent_runtime   (permanent tick error)
    //   running  -> stopped    record_agent_runtime_failure (stop = true; max failures)
    //
    // Tick bookkeeping (`advance_*`, `record_*`) never writes `status` except the last
    // row above. There is deliberately no unconditional "set status" function.

    /// Operator Stop: `running -> stopping` (idempotent for `stopping`). Returns `None`
    /// when the runtime is already stopped.
    pub async fn request_agent_runtime_stop(
        &self,
        runtime_id: Uuid,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = 'stopping', updated_at = now()
            WHERE id = $1 AND status IN ('running','stopping')
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// `stopping -> stopped`, once the last execution is terminal. Never touches a
    /// runtime in any other state.
    pub async fn finish_agent_runtime_stop(
        &self,
        runtime_id: Uuid,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = 'stopped', stopped_at = now(), updated_at = now(), next_run_at = now()
            WHERE id = $1 AND status = 'stopping'
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Platform-initiated stop (`running|stopping -> stopped`) with a reason, used when
    /// the agent no longer exists or is inactive. Cannot move a stopped runtime.
    pub async fn halt_agent_runtime(
        &self,
        runtime_id: Uuid,
        reason: &str,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = 'stopped', last_error = $2, stopped_at = now(), updated_at = now(),
                next_run_at = now()
            WHERE id = $1 AND status IN ('running','stopping')
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(reason)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Permanent tick error (`running -> stopped`). A `stopping` runtime is left for the
    /// stopping branch to finish.
    pub async fn halt_running_agent_runtime(
        &self,
        runtime_id: Uuid,
        reason: &str,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = 'stopped', last_error = $2, stopped_at = now(), updated_at = now(),
                next_run_at = now()
            WHERE id = $1 AND status = 'running'
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(reason)
        .fetch_optional(&self.pool)
        .await?)
    }

    // ---- Tick bookkeeping (never changes status) ----------------------------------

    /// Record an error and the next attempt time without touching `status`. Applies only
    /// to a `running` runtime: a `stopping`/`stopped` runtime keeps its state and
    /// schedule. Returns `None` if the runtime is not running any more.
    pub async fn advance_agent_runtime(
        &self,
        runtime_id: Uuid,
        last_error: Option<&str>,
        next_run_at: DateTime<Utc>,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET last_error = $2, next_run_at = $3, updated_at = now()
            WHERE id = $1 AND status = 'running'
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(last_error)
        .bind(next_run_at)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Record a failed execution: bump the counter, remember which execution it was and
    /// back off. With `stop = true` (failure limit reached) the runtime moves
    /// `running -> stopped` in the same statement. Applies only to a `running` runtime;
    /// returns `None` otherwise, so a concurrent Stop is never overwritten.
    pub async fn record_agent_runtime_failure(
        &self,
        runtime_id: Uuid,
        stop: bool,
        failed_execution_id: Uuid,
        consecutive_failures: i32,
        last_error: &str,
        next_run_at: DateTime<Utc>,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET status = CASE WHEN $2 THEN 'stopped' ELSE status END,
                failed_execution_id = $3,
                consecutive_failures = $4,
                last_error = $5,
                next_run_at = $6,
                updated_at = now(),
                stopped_at = CASE WHEN $2 THEN now() ELSE stopped_at END
            WHERE id = $1 AND status = 'running'
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(stop)
        .bind(failed_execution_id)
        .bind(consecutive_failures)
        .bind(last_error)
        .bind(next_run_at)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Record a newly created execution; never changes `status`. `reset_failures` clears
    /// the failure counter and is set when the previous execution did not fail.
    ///
    /// The tick claimed the runtime while it was `running`, but an operator may have
    /// pressed Stop since. So `last_execution_id` is recorded for `running` and
    /// `stopping` runtimes (the stopping branch waits on, and cancels, that execution),
    /// while the schedule/error/failure fields only change while `running`. Returns
    /// `None` if the runtime is already `stopped`; the caller must then cancel the
    /// orphaned execution.
    pub async fn record_agent_runtime_launch(
        &self,
        runtime_id: Uuid,
        execution_id: Uuid,
        reset_failures: bool,
        next_run_at: DateTime<Utc>,
    ) -> StoreResult<Option<AgentRuntime>> {
        Ok(sqlx::query_as::<_, AgentRuntime>(
            r#"
            UPDATE agent_runtimes
            SET last_execution_id = $2,
                last_error = CASE WHEN status = 'running' THEN NULL ELSE last_error END,
                consecutive_failures = CASE WHEN status = 'running' AND $3 THEN 0 ELSE consecutive_failures END,
                failed_execution_id = CASE WHEN status = 'running' AND $3 THEN NULL ELSE failed_execution_id END,
                next_run_at = CASE WHEN status = 'running' THEN $4 ELSE next_run_at END,
                updated_at = now()
            WHERE id = $1 AND status IN ('running','stopping')
            RETURNING *
            "#,
        )
        .bind(runtime_id)
        .bind(execution_id)
        .bind(reset_failures)
        .bind(next_run_at)
        .fetch_optional(&self.pool)
        .await?)
    }
}
