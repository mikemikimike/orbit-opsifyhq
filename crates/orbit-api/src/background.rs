//! Control-plane maintenance loops: scheduled triggers and orphaned-execution reaping.
//!
//! These used to run inside the worker, which required database access. They are
//! control-plane responsibilities, so they now run in `orbit-api` and keep workers
//! token-only.

use std::time::Duration;

use chrono::Utc;
use orbit_contracts::{subjects, OrbitEvent};
use orbit_domain::{AgentStatus, ExecutionStatus, LogLevel, TriggerSource};
use orbit_store::Trigger;

use crate::state::AppState;

pub fn spawn(state: AppState) {
    // Offline license: re-read the mounted file so airgapped renewals apply without a
    // restart (Secret volume updates propagate within ~60s).
    let license_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            license_state.license.reload();
        }
    });

    let output_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            crate::handlers::outputs::deliver_pending(&output_state).await;
        }
    });

    let scheduler_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(15));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = scheduler_tick(&scheduler_state).await {
                tracing::error!(%error, "scheduler tick failed");
            }
        }
    });

    let reaper_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(15));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            reap(&reaper_state).await;
        }
    });

    let team_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(2));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            match team_state.store.list_running_team_runs().await {
                Ok(runs) => {
                    for run in runs {
                        if let Err(error) = crate::handlers::teams::advance(&team_state, run).await
                        {
                            tracing::warn!(%error, "team run advance failed");
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "team run polling failed"),
            }
        }
    });

    let dispatcher_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(15));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = crate::dispatcher::dispatch_tick(&dispatcher_state).await {
                tracing::error!(%error, "work source dispatcher tick failed");
            }
        }
    });

    let tag_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = reconcile_worker_tags(&tag_state).await {
                tracing::warn!(%error, "worker tag reconciliation failed");
            }
        }
    });

    let runtime_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = runtime_tick(&runtime_state).await {
                tracing::warn!(%error, "agent runtime tick failed");
            }
        }
    });

    let retention_state = state.clone();
    tokio::spawn(async move {
        // Run shortly after start, then daily.
        let mut ticker = tokio::time::interval(Duration::from_secs(6 * 3600));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            archive_old_runs(&retention_state).await;
        }
    });

    spawn_audit_retention(state.clone());

    let pruner_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            match pruner_state.store.prune_expired_memory().await {
                Ok(n) if n > 0 => tracing::info!(count = n, "pruned expired memory entries"),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "memory pruning failed"),
            }
        }
    });
}

/// Default audit-log retention when `ORBIT_AUDIT_RETENTION_DAYS` is unset.
const DEFAULT_AUDIT_RETENTION_DAYS: u32 = 365;

/// Parse `ORBIT_AUDIT_RETENTION_DAYS`: unset/invalid -> default, `0` -> keep forever.
fn audit_retention_days(raw: Option<&str>) -> Option<u32> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Some(DEFAULT_AUDIT_RETENTION_DAYS),
        Some(value) => match value.parse::<u32>() {
            Ok(0) => None,
            Ok(days) => Some(days),
            Err(_) => {
                tracing::warn!(value, "invalid ORBIT_AUDIT_RETENTION_DAYS; using default");
                Some(DEFAULT_AUDIT_RETENTION_DAYS)
            }
        },
    }
}

/// Delete audit log rows older than `ORBIT_AUDIT_RETENTION_DAYS` (default 365, `0` keeps
/// everything). Runs daily; the delete is idempotent so multiple API replicas are safe.
fn spawn_audit_retention(state: AppState) {
    let Some(days) =
        audit_retention_days(std::env::var("ORBIT_AUDIT_RETENTION_DAYS").ok().as_deref())
    else {
        return;
    };
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(24 * 3600));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let cutoff = Utc::now() - chrono::Duration::days(days as i64);
            match state.store.prune_audit_logs(cutoff).await {
                Ok(0) => {}
                Ok(count) => tracing::info!(count, days, "pruned old audit log entries"),
                Err(error) => tracing::warn!(%error, "audit log pruning failed"),
            }
        }
    });
}

/// Archive terminal executions, team runs and worker launches older than the configured
/// retention window. History is kept; the default list simply stops showing old runs.
async fn archive_old_runs(state: &AppState) {
    let Some(days) = state.archive_after_days else {
        return;
    };
    let cutoff = Utc::now() - chrono::Duration::days(days as i64);
    match state.store.archive_executions_before(cutoff).await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, days, "archived old executions"),
        Err(error) => tracing::warn!(%error, "execution archive failed"),
    }
    match state.store.archive_team_runs_before(cutoff).await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, days, "archived old team runs"),
        Err(error) => tracing::warn!(%error, "team run archive failed"),
    }
    match state.store.archive_worker_launches_before(cutoff).await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, days, "archived old worker launches"),
        Err(error) => tracing::warn!(%error, "worker launch archive failed"),
    }
}

async fn scheduler_tick(state: &AppState) -> anyhow::Result<()> {
    for trigger in state.store.claim_due_triggers(10).await? {
        if let Err(error) = fire_trigger(state, trigger).await {
            tracing::warn!(%error, "scheduled trigger failed");
        }
    }
    Ok(())
}

async fn fire_trigger(state: &AppState, trigger: Trigger) -> anyhow::Result<()> {
    let result = fire_trigger_inner(state, trigger.clone()).await;
    match &result {
        Ok(()) => {
            let _ = state.store.record_trigger_success(trigger.id).await;
        }
        Err(error) => {
            tracing::warn!(%error, trigger_id = %trigger.id, "scheduled trigger failed");
            if let Ok(disabled) = state.store.record_trigger_failure(trigger.id).await {
                if disabled {
                    tracing::warn!(trigger_id = %trigger.id, "trigger auto-disabled after consecutive failures");
                }
            }
        }
    }
    result
}

async fn fire_trigger_inner(state: &AppState, trigger: Trigger) -> anyhow::Result<()> {
    let next = trigger
        .config
        .get("cron")
        .and_then(|value| value.as_str())
        .and_then(|cron| cron.parse::<croner::Cron>().ok())
        .and_then(|cron| cron.find_next_occurrence(&Utc::now(), false).ok());
    state.store.reschedule_trigger(trigger.id, next).await?;

    let agent = state
        .store
        .get_agent(trigger.org_id, trigger.agent_id)
        .await?;
    if AgentStatus::parse(&agent.status) != Some(AgentStatus::Active) {
        tracing::debug!(agent_id = %agent.id, "scheduled trigger skipped: agent is not active");
        return Ok(());
    }
    let version = state
        .store
        .get_current_version(trigger.org_id, &agent)
        .await?;
    let spec: orbit_domain::AgentSpec = serde_json::from_value(version.spec.clone())?;

    let active = state
        .store
        .count_active_executions_for_agent(trigger.org_id, agent.id)
        .await?;
    if active >= spec.resources.max_concurrent_executions as i64 {
        tracing::info!(agent_id = %agent.id, "scheduled trigger skipped: concurrency limit");
        return Ok(());
    }

    let fired_at = trigger.last_fired_at.unwrap_or_else(Utc::now);
    // A schedule may carry its own task; otherwise the agent gets a generic prompt.
    let task = trigger
        .config
        .get("task")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|task| !task.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "Scheduled run for agent '{}'. Perform your configured monitoring task.",
                agent.name
            )
        });
    let input = serde_json::json!({
        "task": task,
        "payload": { "triggerId": trigger.id, "firedAt": fired_at },
        "provenance": {
            "source": "schedule",
            "trusted": false,
            "subject": trigger.id,
            "receivedAt": fired_at,
        },
        "agentVersion": version.version,
    });

    let harness_kind = match trigger
        .harness_kind
        .as_deref()
        .and_then(|value| value.parse::<orbit_domain::HarnessKind>().ok())
        .or_else(|| spec.harness.as_ref().map(|harness| harness.kind))
    {
        Some(kind) => kind,
        None => match state.default_harness().await {
            Some(kind) => kind,
            None => {
                tracing::warn!(trigger_id = %trigger.id, "scheduled trigger skipped: no harness configured");
                return Ok(());
            }
        },
    };
    let placement = spec
        .harness
        .as_ref()
        .map(|harness| harness.placement.as_str())
        .unwrap_or("auto");
    let idempotency_key = format!("trigger:{}:{}", trigger.id, fired_at.timestamp_millis());
    let execution = state
        .store
        .create_execution(orbit_store::executions::NewExecution {
            org_id: trigger.org_id,
            agent_id: agent.id,
            agent_version_id: version.id,
            agent_name: &agent.name,
            harness_kind,
            trigger_source: TriggerSource::Schedule,
            placement,
            trigger_id: Some(trigger.id),
            input,
            session_ref: None,
            skills: &[],
            timeout_seconds: spec.resources.timeout_seconds,
            requested_by: None,
            idempotency_key: Some(idempotency_key.as_str()),
            team_run_id: None,
            team_step_index: None,
        })
        .await?;

    let event = OrbitEvent::new(
        subjects::EXECUTION_REQUESTED,
        "orbit-api-scheduler",
        serde_json::json!({
            "executionId": execution.id,
            "agentId": agent.id,
            "triggerId": trigger.id,
            "harness": harness_kind.as_str(),
        }),
    )
    .with_org(trigger.org_id)
    .with_idempotency(format!("exec:{}", execution.id));
    if let Err(error) = state.bus.publish(event).await {
        tracing::warn!(%error, "scheduled execution publish failed; polling will pick it up");
    }
    tracing::info!(
        execution_id = %execution.id,
        agent_id = %agent.id,
        status = ExecutionStatus::Queued.as_str(),
        "scheduled execution queued"
    );
    Ok(())
}

/// A worker whose heartbeat stops for this long is considered gone.
const HEARTBEAT_STALE_SECONDS: i64 = 90;
/// A claimed run with no activity for this long was never started by its worker. Long
/// enough for a large repository checkout, which reports only when it finishes.
const UNSTARTED_SILENT_SECONDS: i64 = 600;
/// How long a claimed, never-started run may ignore a cancel request.
const UNSTARTED_CANCEL_SECONDS: i64 = 60;

async fn reap(state: &AppState) {
    match state
        .store
        .reap_orphaned_executions(HEARTBEAT_STALE_SECONDS)
        .await
    {
        Ok(rows) => {
            for execution in rows {
                tracing::warn!(
                    execution_id = %execution.id,
                    worker_id = ?execution.worker_id,
                    "reaped orphaned execution"
                );
                report_reaped(
                    state,
                    &execution,
                    "worker heartbeat expired; execution marked failed",
                    &orbit_store::workers::worker_lost_error(),
                )
                .await;
            }
        }
        Err(error) => tracing::warn!(%error, "execution reaper failed"),
    }

    match state
        .store
        .reap_unstarted_executions(None, UNSTARTED_SILENT_SECONDS, UNSTARTED_CANCEL_SECONDS)
        .await
    {
        Ok(rows) => {
            for execution in rows {
                let cancelled = execution.status == ExecutionStatus::Cancelled.as_str();
                tracing::warn!(
                    execution_id = %execution.id,
                    worker_id = ?execution.worker_id,
                    status = %execution.status,
                    "ended execution its worker never started"
                );
                let message = if cancelled {
                    "cancelled; the worker that took this execution never started it"
                } else {
                    "a worker took this execution but never started it; execution marked failed"
                };
                let data = execution.error.clone().unwrap_or_else(
                    || serde_json::json!({ "code": "worker_did_not_start", "retryable": true }),
                );
                report_reaped(state, &execution, message, &data).await;
            }
        }
        Err(error) => tracing::warn!(%error, "unstarted execution reaper failed"),
    }
}

/// Record why the platform ended an execution and tell subscribers.
async fn report_reaped(
    state: &AppState,
    execution: &orbit_store::ExecutionRecord,
    message: &str,
    data: &serde_json::Value,
) {
    let _ = state
        .store
        .append_execution_event(
            execution.org_id,
            execution.id,
            "error",
            LogLevel::Error,
            message,
            data,
        )
        .await;
    let event = OrbitEvent::new(
        subjects::EXECUTION_UPDATED,
        "orbit-api-reaper",
        serde_json::json!({
            "executionId": execution.id,
            "agentId": execution.agent_id,
            "status": execution.status,
        }),
    )
    .with_org(execution.org_id)
    .with_idempotency(format!("reap:{}", execution.id));
    let _ = state.bus.publish(event).await;
}

/// Relaunch platform-follow worker profiles when ORBIT_IMAGE_TAG changes. The old token
/// is revoked first (no new claims), active executions drain, then the launcher stops and
/// replaces the worker.
async fn reconcile_worker_tags(state: &AppState) -> anyhow::Result<()> {
    if state.launcher_url.is_none() {
        return Ok(());
    }
    let tag = crate::images::image_tag();
    let profiles = state.store.list_platform_follow_profiles().await?;
    for profile in profiles {
        let Some(user_id) = profile.owner_user_id else {
            continue;
        };
        let desired = format!("{}:{}", profile.image, tag);
        let launches = state
            .store
            .list_worker_launches(profile.org_id, None)
            .await?;
        for launch in launches.into_iter().filter(|launch| {
            launch.profile_id == profile.id
                && matches!(launch.status.as_str(), "pending" | "starting" | "running")
                && launch.image != desired
        }) {
            let auth = orbit_security::AuthContext {
                user_id,
                org_id: profile.org_id,
                email: "system@orbit.local".into(),
                display_name: "ORBIT auto-update".into(),
                role: orbit_domain::UserRole::Owner,
                method: orbit_security::AuthMethod::ApiToken,
                scopes: Vec::new(),
                token_id: None,
            };
            if let Some(token_id) = launch.worker_token_id {
                let _ = state
                    .store
                    .revoke_worker_token(profile.org_id, token_id)
                    .await;
            }
            if let Some(worker_id) = &launch.worker_id {
                if state
                    .store
                    .has_active_executions_for_worker(profile.org_id, worker_id)
                    .await?
                {
                    tracing::info!(profile = %profile.name, %worker_id, "worker update waiting for drain");
                    continue;
                }
            }
            let old_image = launch.image.clone();
            match crate::handlers::worker_launches::stop_launch_internal(state, &auth, launch).await
            {
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(profile = %profile.name, %error, "could not stop old worker");
                    continue;
                }
            }
            match crate::handlers::worker_launches::launch_profile_internal(
                state, &auth, profile.id, None,
            )
            .await
            {
                Ok(new_launch) => {
                    tracing::info!(
                        profile = %profile.name,
                        old = %old_image,
                        new = %new_launch.image,
                        "worker auto-relaunched for platform tag"
                    );
                    crate::audit::record(
                        state,
                        &auth,
                        "worker.auto_relaunched",
                        Some("worker_profile"),
                        Some(profile.id),
                        serde_json::json!({
                            "oldImage": old_image,
                            "newImage": new_launch.image,
                            "launchId": new_launch.id,
                        }),
                    )
                    .await;
                }
                Err(error) => {
                    tracing::error!(profile = %profile.name, %error, "worker auto-relaunch failed");
                }
            }
        }
    }
    Ok(())
}

/// Advance continuous agent runtimes: create the next execution when the previous one
/// is terminal, and stop runtimes whose agent is no longer active.
async fn runtime_tick(state: &AppState) -> anyhow::Result<()> {
    for runtime in state.store.list_running_agent_runtimes().await? {
        let _ = state.store.heartbeat_agent_runtime(runtime.id).await;
        let agent = match state
            .store
            .get_agent(runtime.org_id, runtime.agent_id)
            .await
        {
            Ok(agent) => agent,
            Err(_) => {
                let _ = state
                    .store
                    .update_agent_runtime(
                        runtime.id,
                        "stopped",
                        runtime.last_execution_id,
                        Some("agent no longer exists"),
                        chrono::Utc::now(),
                    )
                    .await;
                continue;
            }
        };
        if agent.status != "active" {
            let _ = state
                .store
                .update_agent_runtime(
                    runtime.id,
                    "stopped",
                    runtime.last_execution_id,
                    Some("agent is not active"),
                    chrono::Utc::now(),
                )
                .await;
            continue;
        }

        let last = match runtime.last_execution_id {
            Some(id) => state.store.get_execution(runtime.org_id, id).await.ok(),
            None => None,
        };
        let last_terminal = last.as_ref().is_none_or(|execution| {
            matches!(
                ExecutionStatus::parse(&execution.status),
                Some(status) if status.is_terminal()
            )
        });
        if runtime.status == "stopping" {
            if last_terminal {
                let _ = state
                    .store
                    .update_agent_runtime(
                        runtime.id,
                        "stopped",
                        runtime.last_execution_id,
                        runtime.last_error.as_deref(),
                        chrono::Utc::now(),
                    )
                    .await;
            }
            continue;
        }
        if !last_terminal {
            continue;
        }
        let Some(runtime) = state
            .store
            .claim_agent_runtime_tick(
                runtime.id,
                chrono::Utc::now() + chrono::Duration::seconds(5),
            )
            .await?
        else {
            continue;
        };
        if last
            .as_ref()
            .is_some_and(|execution| execution.status == "failed")
        {
            let _ = state
                .store
                .update_agent_runtime(
                    runtime.id,
                    "running",
                    runtime.last_execution_id,
                    last.as_ref()
                        .and_then(|execution| execution.error.as_ref())
                        .map(|error| error.to_string())
                        .as_deref(),
                    chrono::Utc::now()
                        + chrono::Duration::seconds(runtime.interval_seconds.max(30) as i64),
                )
                .await;
            continue;
        }

        let version = state
            .store
            .get_current_version(runtime.org_id, &agent)
            .await?;
        let spec: orbit_domain::AgentSpec = serde_json::from_value(version.spec.clone())?;
        let harness_kind = runtime
            .harness_kind
            .parse::<orbit_domain::HarnessKind>()
            .map_err(|_| anyhow::anyhow!("runtime has an unknown harness"))?;
        let input = serde_json::json!({
            "task": runtime.task,
            "payload": { "runtimeId": runtime.id },
            "provenance": {
                "source": "runtime",
                "trusted": true,
                "subject": runtime.id,
                "receivedAt": Utc::now(),
            },
        });
        let execution = state
            .store
            .create_execution(orbit_store::executions::NewExecution {
                org_id: runtime.org_id,
                agent_id: agent.id,
                agent_version_id: version.id,
                agent_name: &agent.name,
                harness_kind,
                trigger_source: TriggerSource::Manual,
                placement: spec
                    .harness
                    .as_ref()
                    .map(|harness| harness.placement.as_str())
                    .unwrap_or("auto"),
                trigger_id: None,
                input,
                session_ref: last
                    .as_ref()
                    .and_then(|execution| execution.session_ref.as_deref()),
                skills: &[],
                timeout_seconds: spec.resources.timeout_seconds,
                requested_by: runtime.created_by,
                idempotency_key: Some(&format!("runtime:{}:{}", runtime.id, uuid::Uuid::now_v7())),
                team_run_id: None,
                team_step_index: None,
            })
            .await?;
        state
            .store
            .update_agent_runtime(
                runtime.id,
                "running",
                Some(execution.id),
                None,
                chrono::Utc::now() + chrono::Duration::seconds(runtime.interval_seconds as i64),
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::audit_retention_days;

    #[test]
    fn audit_retention_parsing() {
        assert_eq!(audit_retention_days(None), Some(365));
        assert_eq!(audit_retention_days(Some("")), Some(365));
        assert_eq!(audit_retention_days(Some("90")), Some(90));
        assert_eq!(audit_retention_days(Some("0")), None);
        assert_eq!(audit_retention_days(Some("abc")), Some(365));
    }
}
