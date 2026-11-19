//! Execution result outputs (Slack/Mattermost/webhook) and delivery.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Extension;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use orbit_domain::ValidationIssue;
use orbit_security::{new_secret_ref, AuthContext, Permission, SecretValue};
use orbit_store::{ExecutionRecord, Output, TeamRun};

use crate::audit;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputRequest {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub agent_id: Option<Uuid>,
    #[serde(default)]
    pub team_id: Option<Uuid>,
    #[serde(default)]
    pub events: Vec<String>,
    /// Incoming webhook / target URL. Write-only; never returned.
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutputView {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub agent_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    pub events: Vec<String>,
    pub template: Option<String>,
    pub enabled: bool,
    pub has_webhook: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<Output> for OutputView {
    fn from(output: Output) -> Self {
        Self {
            id: output.id,
            name: output.name,
            kind: output.kind,
            agent_id: output.agent_id,
            team_id: output.team_id,
            events: output.events,
            template: output.template,
            enabled: output.enabled,
            has_webhook: !output.secret_ref.is_empty(),
            created_at: output.created_at,
            updated_at: output.updated_at,
        }
    }
}

fn validated(body: &OutputRequest) -> Result<(String, Vec<String>), ApiError> {
    let mut issues = Vec::new();
    let name = body.name.trim().to_string();
    if body.agent_id.is_some() && body.team_id.is_some() {
        issues.push(ValidationIssue::new(
            "target",
            "choose an agent or a team, not both",
        ));
    }
    if name.is_empty() {
        issues.push(ValidationIssue::new("name", "name is required"));
    }
    if !matches!(body.kind.as_str(), "slack" | "mattermost" | "webhook") {
        issues.push(ValidationIssue::new(
            "kind",
            "kind must be slack, mattermost or webhook",
        ));
    }
    let mut events = if body.events.is_empty() {
        vec!["succeeded".into(), "failed".into(), "cancelled".into()]
    } else {
        body.events.clone()
    };
    events.sort();
    events.dedup();
    for event in &events {
        if !matches!(event.as_str(), "succeeded" | "failed" | "cancelled") {
            issues.push(ValidationIssue::new(
                "events",
                format!("unknown event '{event}'"),
            ));
        }
    }
    if !issues.is_empty() {
        return Err(ApiError::validation(issues));
    }
    Ok((name, events))
}

#[utoipa::path(get, path = "/api/v1/outputs", tag = "outputs", responses((status = 200, body = [OutputView])))]
pub async fn list(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<OutputView>>, ApiError> {
    auth.require(Permission::SettingsRead)?;
    Ok(Json(
        state
            .store
            .list_outputs(auth.org_id)
            .await?
            .into_iter()
            .map(OutputView::from)
            .collect(),
    ))
}

#[utoipa::path(post, path = "/api/v1/outputs", tag = "outputs", request_body = OutputRequest, responses((status = 201, body = OutputView)))]
pub async fn create(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<OutputRequest>,
) -> Result<(StatusCode, Json<OutputView>), ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let (name, events) = validated(&body)?;
    let url = body
        .webhook_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::validation(vec![ValidationIssue::new(
                "webhookUrl",
                "webhookUrl is required",
            )])
        })?;
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(ApiError::validation(vec![ValidationIssue::new(
            "webhookUrl",
            "webhookUrl must be http(s)",
        )]));
    }
    guard_webhook_url(url).await?;
    let secret_ref = new_secret_ref("output");
    state
        .store
        .put_secret(
            Some(auth.org_id),
            &secret_ref,
            &SecretValue::new(url),
            &format!("output:{name}"),
        )
        .await?;
    let output = state
        .store
        .create_output(
            auth.org_id,
            &name,
            &body.kind,
            body.agent_id,
            body.team_id,
            &events,
            &secret_ref,
            body.template.as_deref(),
        )
        .await?;
    audit::record(
        &state,
        &auth,
        "output.created",
        Some("output"),
        Some(output.id),
        serde_json::json!({ "name": output.name, "kind": output.kind }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(OutputView::from(output))))
}

#[utoipa::path(patch, path = "/api/v1/outputs/{output_id}", tag = "outputs", request_body = OutputRequest, responses((status = 200, body = OutputView)))]
pub async fn update(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(output_id): Path<Uuid>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<OutputRequest>,
) -> Result<Json<OutputView>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let (name, events) = validated(&body)?;
    let secret_ref = match body.webhook_url.as_deref().map(str::trim) {
        Some(url) if !url.is_empty() => {
            if !url.starts_with("https://") && !url.starts_with("http://") {
                return Err(ApiError::validation(vec![ValidationIssue::new(
                    "webhookUrl",
                    "webhookUrl must be http(s)",
                )]));
            }
            guard_webhook_url(url).await?;
            let reference = new_secret_ref("output");
            state
                .store
                .put_secret(
                    Some(auth.org_id),
                    &reference,
                    &SecretValue::new(url),
                    &format!("output:{name}"),
                )
                .await?;
            Some(reference)
        }
        _ => None,
    };
    let output = state
        .store
        .update_output(
            auth.org_id,
            output_id,
            &name,
            &body.kind,
            body.agent_id,
            body.team_id,
            &events,
            secret_ref.as_deref(),
            body.template.as_deref(),
            body.enabled,
        )
        .await?;
    audit::record(
        &state,
        &auth,
        "output.updated",
        Some("output"),
        Some(output.id),
        serde_json::json!({ "name": output.name, "kind": output.kind }),
    )
    .await;
    Ok(Json(OutputView::from(output)))
}

#[utoipa::path(delete, path = "/api/v1/outputs/{output_id}", tag = "outputs", responses((status = 204)))]
pub async fn delete(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(output_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    state.store.delete_output(auth.org_id, output_id).await?;
    audit::record(
        &state,
        &auth,
        "output.deleted",
        Some("output"),
        Some(output_id),
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v1/outputs/{output_id}/test", tag = "outputs", responses((status = 200)))]
pub async fn test_output(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(output_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let output = state.store.get_output(auth.org_id, output_id).await?;
    let text = "ORBIT test notification — your output is configured correctly.";
    deliver(&state, &output, text, None, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delivery used by the test endpoint and the background outbox worker.
pub async fn deliver(
    state: &AppState,
    output: &Output,
    text: &str,
    execution: Option<&ExecutionRecord>,
    team_run: Option<&TeamRun>,
) -> Result<(), ApiError> {
    let url = state
        .store
        .get_secret_value(&output.secret_ref)
        .await?
        .map(|value| value.expose().to_string())
        .ok_or_else(|| ApiError::internal("output webhook secret is missing"))?;
    guard_webhook_url(&url).await.map_err(|error| {
        ApiError::new(
            StatusCode::BAD_GATEWAY,
            "output_delivery_failed",
            error.message.clone(),
        )
    })?;
    let client = orbit_security::guarded_client(std::time::Duration::from_secs(15));
    let payload = if output.kind == "webhook" {
        serde_json::json!({
            "event": execution.map(|e| format!("execution.{}", e.status)).unwrap_or_else(|| "test".into()),
            "text": text,
            "execution": execution.map(|e| serde_json::json!({
                "id": e.id, "agentName": e.agent_name, "status": e.status,
                "output": e.output, "error": e.error,
            })),
            "teamRun": team_run.map(|run| serde_json::json!({
                "id": run.id, "teamId": run.team_id, "status": run.status,
                "output": run.output, "error": run.error,
            })),
        })
    } else {
        serde_json::json!({ "text": text })
    };
    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "output_delivery_failed",
                orbit_security::describe_request_error(&error),
            )
        })?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "output_delivery_failed",
            format!("webhook returned HTTP {}", response.status()),
        ));
    }
    Ok(())
}

/// Background outbox loop: enqueue recent terminal executions, deliver due rows.
pub async fn deliver_pending(state: &AppState) {
    let since = chrono::Utc::now() - chrono::Duration::minutes(30);
    let _ = state.store.enqueue_output_deliveries(since).await;
    let _ = state.store.enqueue_team_output_deliveries(since).await;
    let Ok(jobs) = state.store.due_output_deliveries(25).await else {
        return;
    };
    for job in jobs {
        let output = match state.store.get_output(job.org_id, job.output_id).await {
            Ok(output) => output,
            Err(_) => continue,
        };
        let execution = match job.execution_id {
            Some(id) => state.store.get_execution_by_id_any_org(id).await.ok(),
            None => None,
        };
        let team_run = match job.team_run_id {
            Some(id) => state.store.get_team_run_any_org(id).await.ok(),
            None => None,
        };
        let text = execution
            .as_ref()
            .map(summary)
            .or_else(|| team_run.as_ref().map(team_summary))
            .unwrap_or_else(|| "ORBIT run finished".into());
        let result = deliver(state, &output, &text, execution.as_ref(), team_run.as_ref()).await;
        let attempts = job.attempts + 1;
        let (success, status, error) = match &result {
            Ok(()) => (true, None, None),
            Err(error) => (false, None, Some(error.message.clone())),
        };
        let _ = state
            .store
            .finish_output_delivery(job.id, success, status, error.as_deref(), attempts)
            .await;
    }
}

fn team_summary(run: &TeamRun) -> String {
    let output = run
        .output
        .as_ref()
        .map(|value| {
            value
                .get("text")
                .or_else(|| value.get("summary"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .or_else(|| run.error.clone())
        .unwrap_or_else(|| "completed".into());
    let output: String = output.chars().take(3000).collect();
    format!(
        "Agent team run {} — {}\n{}",
        run.status,
        run.input
            .get("task")
            .and_then(|v| v.as_str())
            .unwrap_or("task"),
        output
    )
}

fn summary(execution: &ExecutionRecord) -> String {
    let output = execution
        .output
        .as_ref()
        .map(|value| {
            value
                .get("text")
                .or_else(|| value.get("summary"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string())
        })
        .or_else(|| execution.error.as_ref().map(|error| error.to_string()))
        .unwrap_or_else(|| "completed".into());
    let output: String = output.chars().take(3000).collect();
    format!(
        "{} {} — {}\\n{}",
        execution.agent_name,
        execution.status,
        execution
            .input
            .get("task")
            .and_then(|v| v.as_str())
            .unwrap_or("task"),
        output
    )
}

/// Refuse webhook targets in loopback, link-local/metadata or other blocked ranges
/// (see `ORBIT_OUTBOUND_*`). Private ranges are allowed by default.
async fn guard_webhook_url(url: &str) -> Result<(), ApiError> {
    orbit_security::check_outbound_url(url, &orbit_security::OutboundPolicy::from_env())
        .await
        .map_err(|error| {
            ApiError::validation(vec![ValidationIssue::new("webhookUrl", error.to_string())])
        })
}
