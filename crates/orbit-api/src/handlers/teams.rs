//! Agent teams: sequential role handoffs materialized as normal executions.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Extension;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use orbit_domain::{HarnessKind, TriggerSource, ValidationIssue};
use orbit_security::{AuthContext, Permission};
use orbit_store::{AgentTeam, TeamRun, TeamStep};

use crate::dto::ArchiveListQuery;
use crate::error::ApiError;
use crate::state::AppState;
use crate::work_sources::{
    validate_repositories, validate_work_source, RepositorySpec, WorkSourceDefinition,
};

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamStepInput {
    pub agent_id: Uuid,
    #[serde(default)]
    pub model: Option<TeamModelOverride>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `pipeline` runs each agent once in order. `native` runs the primary agent only
    /// and materializes the remaining agents as harness-native subagents.
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Shared goal/instruction for the team. The first step receives it as its task;
    /// later steps receive it together with the previous step output.
    #[serde(default)]
    pub instructions: String,
    /// Structured workspace repository (owner/repo). Instructions alone cannot drive
    /// repository checkout; this is passed to the worker.
    #[serde(default)]
    pub repository: Option<String>,
    /// Optional base/working branch for repository checkout.
    #[serde(default)]
    pub branch: Option<String>,
    /// Team-level model override (optional).
    #[serde(default)]
    pub model: Option<TeamModelOverride>,
    /// Automatic issue discovery/claiming configuration.
    #[serde(default)]
    pub work_source: Option<WorkSourceDefinition>,
    /// Multi-repository workspace. When empty, `repository`/`branch` are used.
    #[serde(default)]
    pub repositories: Vec<RepositorySpec>,
    /// Worker profile used for automatic launch.
    #[serde(default)]
    pub worker_profile_id: Option<Uuid>,
    #[serde(default)]
    pub auto_launch: bool,
    /// Skills materialized for every step in the team.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Harness for team steps. Falls back to the legacy agent harness or org default.
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub connection_ids: Vec<Uuid>,
    #[serde(default)]
    pub env_mapping_ids: Vec<Uuid>,
    /// Per-step wall-clock timeout; defaults to 1 hour for team runs.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    pub steps: Vec<TeamStepInput>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTeamRunRequest {
    /// Optional run-specific task. Falls back to the team instruction.
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    /// Explicitly allow bound writes for this run without per-call approvals (L4).
    #[serde(default)]
    pub auto_approve: bool,
    /// Override the agent's stored harness for this run.
    #[serde(default)]
    pub harness_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamRunDetail {
    pub run: TeamRun,
    pub steps: Vec<TeamStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamDefinition {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub work_source: Option<WorkSourceDefinition>,
    #[serde(default)]
    pub repositories: Vec<RepositorySpec>,
    #[serde(default)]
    pub worker_profile_id: Option<Uuid>,
    #[serde(default)]
    pub auto_launch: bool,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub harness: Option<String>,
    #[serde(default)]
    pub connection_ids: Vec<Uuid>,
    #[serde(default)]
    pub env_mapping_ids: Vec<Uuid>,
    #[serde(default)]
    pub model: Option<TeamModelOverride>,
    /// Wall-clock timeout for each step. Defaults to 1 hour for team runs.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    pub steps: Vec<TeamStepDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TeamModelOverride {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub management: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamStepDefinition {
    pub agent_id: Uuid,
    #[serde(default)]
    pub model: Option<TeamModelOverride>,
}

fn default_mode() -> String {
    "pipeline".into()
}

pub(crate) fn parse_definition(team: &AgentTeam) -> Result<TeamDefinition, ApiError> {
    serde_json::from_value(team.definition.clone())
        .map_err(|error| ApiError::internal(format!("team definition is invalid: {error}")))
}

/// Connections a team run injects beyond the agent specs: the team's shared connections
/// and an enabled work source connection.
pub(crate) fn team_connection_ids(definition: &TeamDefinition) -> Vec<Uuid> {
    let mut ids = definition.connection_ids.clone();
    if let Some(source) = &definition.work_source {
        if source.enabled {
            ids.extend(source.connection_id);
        }
    }
    ids
}

/// A caller-supplied repository must be one of the team's configured repositories; the
/// run injects the team's connection credentials, so it must not retarget them.
fn validate_requested_repositories(
    definition: &TeamDefinition,
    payload: &serde_json::Value,
) -> Result<(), ApiError> {
    let mut allowed: Vec<String> = Vec::new();
    allowed.extend(definition.repository.iter().cloned());
    allowed.extend(
        definition
            .repositories
            .iter()
            .map(|repository| repository.repository.clone()),
    );
    if let Some(source) = &definition.work_source {
        allowed.extend(source.repository.iter().cloned());
    }
    let is_allowed = |value: &str| {
        allowed
            .iter()
            .any(|candidate| candidate.trim().eq_ignore_ascii_case(value.trim()))
    };
    if let Some(repository) = payload.get("repository").and_then(|value| value.as_str()) {
        if !is_allowed(repository) {
            return Err(ApiError::validation(vec![ValidationIssue::new(
                "payload.repository",
                "repository is not one of the team's configured repositories",
            )]));
        }
    }
    if let Some(items) = payload
        .get("repositories")
        .and_then(|value| value.as_array())
    {
        for (idx, item) in items.iter().enumerate() {
            let repository = item
                .get("repository")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if !is_allowed(repository) {
                return Err(ApiError::validation(vec![ValidationIssue::new(
                    format!("payload.repositories[{idx}].repository"),
                    "repository is not one of the team's configured repositories",
                )]));
            }
        }
    }
    Ok(())
}

async fn validate_work_source_connection(
    state: &AppState,
    org_id: Uuid,
    source: &WorkSourceDefinition,
    issues: &mut Vec<ValidationIssue>,
) {
    if !source.enabled {
        return;
    }
    let Some(connection_id) = source.connection_id else {
        return;
    };
    match state
        .store
        .get_connection_with_connector(org_id, connection_id)
        .await
    {
        Ok(connection) => {
            if connection.connector_slug != source.provider {
                issues.push(ValidationIssue::new(
                    "workSource.connectionId",
                    format!(
                        "connection belongs to '{}', not '{}'",
                        connection.connector_slug, source.provider
                    ),
                ));
            } else if connection.connection.status != "active" {
                issues.push(ValidationIssue::new(
                    "workSource.connectionId",
                    format!("connection '{}' is not active", connection.connection.name),
                ));
            }
        }
        Err(_) => issues.push(ValidationIssue::new(
            "workSource.connectionId",
            format!("unknown connection '{connection_id}'"),
        )),
    }
}

fn work_source_to_json(source: &WorkSourceDefinition) -> serde_json::Value {
    serde_json::to_value(source).unwrap_or(serde_json::Value::Null)
}

fn repositories_to_json(repositories: &[RepositorySpec]) -> serde_json::Value {
    serde_json::json!(repositories)
}

async fn validate_native_primary(
    state: &AppState,
    org_id: Uuid,
    mode: &str,
    steps: &[TeamStepInput],
    issues: &mut Vec<ValidationIssue>,
) {
    if mode != "native" {
        return;
    }
    let Some(primary) = steps.first() else {
        return;
    };
    let Ok(agent) = state.store.get_agent(org_id, primary.agent_id).await else {
        return;
    };
    let Ok(version) = state.store.get_current_version(org_id, &agent).await else {
        return;
    };
    match serde_json::from_value::<orbit_domain::AgentSpec>(version.spec) {
        Ok(spec)
            if spec
                .harness
                .as_ref()
                .map(|harness| harness.kind)
                .unwrap_or(HarnessKind::OpenCode)
                == HarnessKind::OpenCode => {}
        Ok(_) => issues.push(ValidationIssue::new(
            "mode",
            "native Agent Teams currently require an OpenCode primary agent; Pi and other harnesses can use pipeline mode",
        )),
        Err(error) => issues.push(ValidationIssue::new(
            "mode",
            format!("primary agent spec is invalid: {error}"),
        )),
    }
}

async fn require_team_view(
    state: &AppState,
    auth: &AuthContext,
    team_id: Uuid,
) -> Result<orbit_store::AgentTeam, ApiError> {
    let team = state.store.get_agent_team(auth.org_id, team_id).await?;
    let access = crate::access::AccessScope::load(state, auth).await?;
    access.require_view(
        "agent team",
        team.owner_user_id,
        team.owner_team_id,
        &team.visibility,
    )?;
    Ok(team)
}

async fn require_team_manage(
    state: &AppState,
    auth: &AuthContext,
    team_id: Uuid,
) -> Result<orbit_store::AgentTeam, ApiError> {
    let team = state.store.get_agent_team(auth.org_id, team_id).await?;
    let access = crate::access::AccessScope::load(state, auth).await?;
    access.require_manage(
        "agent team",
        team.owner_user_id,
        team.owner_team_id,
        &team.visibility,
    )?;
    Ok(team)
}

#[utoipa::path(post, path = "/api/v1/agent-teams", tag = "agent-teams", request_body = CreateTeamRequest, responses((status = 201, body = orbit_store::AgentTeam)))]
pub async fn create_team(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<CreateTeamRequest>,
) -> Result<(StatusCode, Json<AgentTeam>), ApiError> {
    auth.require(Permission::SettingsWrite)?;
    if body.name.trim().is_empty() || body.steps.is_empty() {
        return Err(ApiError::validation(vec![
            ValidationIssue::new("name", "name is required"),
            ValidationIssue::new("steps", "at least one step is required"),
        ]));
    }
    if !matches!(body.mode.as_str(), "pipeline" | "native") {
        return Err(ApiError::validation(vec![ValidationIssue::new(
            "mode",
            "mode must be pipeline or native",
        )]));
    }
    if body
        .timeout_seconds
        .is_some_and(|seconds| !(1..=86_400).contains(&seconds))
    {
        return Err(ApiError::validation(vec![ValidationIssue::new(
            "timeoutSeconds",
            "must be 1..=86400",
        )]));
    }
    if let Some(repository) = body
        .repository
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        if !repository.contains('/') || repository.chars().any(char::is_whitespace) {
            return Err(ApiError::validation(vec![ValidationIssue::new(
                "repository",
                "repository must be owner/repo",
            )]));
        }
    }
    let mut issues = Vec::new();
    if let Some(source) = &body.work_source {
        issues.extend(validate_work_source(source));
    }
    if !body.repositories.is_empty() {
        issues.extend(validate_repositories(&body.repositories));
    }
    let mut agent_ids = std::collections::BTreeSet::new();
    for step in &body.steps {
        if !agent_ids.insert(step.agent_id) {
            issues.push(ValidationIssue::new(
                "steps.agentId",
                "the same agent cannot appear twice in one team",
            ));
        }
        if state
            .store
            .get_agent(auth.org_id, step.agent_id)
            .await
            .is_err()
        {
            issues.push(ValidationIssue::new(
                "steps.agentId",
                format!("unknown agent '{}'", step.agent_id),
            ));
        }
    }
    for slug in &body.skills {
        if state
            .store
            .get_skill_by_slug(auth.org_id, slug)
            .await?
            .is_none()
        {
            issues.push(ValidationIssue::new(
                "skills",
                format!("unknown skill '{slug}'"),
            ));
        }
    }
    if let Some(source) = &body.work_source {
        validate_work_source_connection(&state, auth.org_id, source, &mut issues).await;
    }
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    for connection_id in &body.connection_ids {
        match state
            .store
            .get_connection(auth.org_id, *connection_id)
            .await
        {
            Ok(connection)
                if access.can_view(
                    connection.owner_user_id,
                    connection.owner_team_id,
                    &connection.visibility,
                ) => {}
            Ok(_) => issues.push(ValidationIssue::new(
                "connectionIds",
                format!("connection '{connection_id}' is not visible to you"),
            )),
            Err(_) => issues.push(ValidationIssue::new(
                "connectionIds",
                format!("unknown connection '{connection_id}'"),
            )),
        }
    }
    for mapping_id in &body.env_mapping_ids {
        if state
            .store
            .get_env_mapping(auth.org_id, *mapping_id)
            .await
            .is_err()
        {
            issues.push(ValidationIssue::new(
                "envMappingIds",
                format!("unknown environment mapping '{mapping_id}'"),
            ));
        }
    }
    validate_native_primary(&state, auth.org_id, &body.mode, &body.steps, &mut issues).await;
    if !issues.is_empty() {
        return Err(ApiError::validation(issues));
    }
    let definition = serde_json::json!({
        "mode": body.mode,
        "instructions": body.instructions.trim(),
        "repository": body.repository.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        "branch": body.branch.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        "workSource": body.work_source.as_ref().map(work_source_to_json),
        "repositories": repositories_to_json(&body.repositories),
        "model": body.model,
        "workerProfileId": body.worker_profile_id,
        "autoLaunch": body.auto_launch,
        "skills": body.skills,
        "harness": body.harness,
        "connectionIds": body.connection_ids,
        "envMappingIds": body.env_mapping_ids,
        "timeoutSeconds": body.timeout_seconds,
        "steps": body.steps.iter().map(|step| serde_json::json!({
            "agentId": step.agent_id,
            "model": step.model,
        })).collect::<Vec<_>>()
    });
    let team = state
        .store
        .create_agent_team(
            auth.org_id,
            body.name.trim(),
            body.description.trim(),
            &definition,
            Some(auth.user_id),
        )
        .await?;
    state
        .store
        .assign_resource_owner(auth.org_id, "agent_teams", team.id, auth.user_id)
        .await?;
    Ok((StatusCode::CREATED, Json(team)))
}

#[utoipa::path(patch, path = "/api/v1/agent-teams/{team_id}", tag = "agent-teams", request_body = CreateTeamRequest, responses((status = 200, body = orbit_store::AgentTeam)))]
pub async fn update_team(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(team_id): Path<Uuid>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<CreateTeamRequest>,
) -> Result<Json<AgentTeam>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    require_team_manage(&state, &auth, team_id).await?;
    if body.name.trim().is_empty() || body.steps.is_empty() {
        return Err(ApiError::validation(vec![
            ValidationIssue::new("name", "name is required"),
            ValidationIssue::new("steps", "at least one step is required"),
        ]));
    }
    if !matches!(body.mode.as_str(), "pipeline" | "native") {
        return Err(ApiError::validation(vec![ValidationIssue::new(
            "mode",
            "mode must be pipeline or native",
        )]));
    }
    if body
        .timeout_seconds
        .is_some_and(|seconds| !(1..=86_400).contains(&seconds))
    {
        return Err(ApiError::validation(vec![ValidationIssue::new(
            "timeoutSeconds",
            "must be 1..=86400",
        )]));
    }
    if let Some(repository) = body
        .repository
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        if !repository.contains('/') || repository.chars().any(char::is_whitespace) {
            return Err(ApiError::validation(vec![ValidationIssue::new(
                "repository",
                "repository must be owner/repo",
            )]));
        }
    }
    let mut issues = Vec::new();
    if let Some(source) = &body.work_source {
        issues.extend(validate_work_source(source));
    }
    if !body.repositories.is_empty() {
        issues.extend(validate_repositories(&body.repositories));
    }
    let mut agent_ids = std::collections::BTreeSet::new();
    for step in &body.steps {
        if !agent_ids.insert(step.agent_id) {
            issues.push(ValidationIssue::new(
                "steps.agentId",
                "the same agent cannot appear twice in one team",
            ));
        }
        if state
            .store
            .get_agent(auth.org_id, step.agent_id)
            .await
            .is_err()
        {
            issues.push(ValidationIssue::new(
                "steps.agentId",
                format!("unknown agent '{}'", step.agent_id),
            ));
        }
    }
    for slug in &body.skills {
        if state
            .store
            .get_skill_by_slug(auth.org_id, slug)
            .await?
            .is_none()
        {
            issues.push(ValidationIssue::new(
                "skills",
                format!("unknown skill '{slug}'"),
            ));
        }
    }
    if let Some(source) = &body.work_source {
        validate_work_source_connection(&state, auth.org_id, source, &mut issues).await;
    }
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    for connection_id in &body.connection_ids {
        match state
            .store
            .get_connection(auth.org_id, *connection_id)
            .await
        {
            Ok(connection)
                if access.can_view(
                    connection.owner_user_id,
                    connection.owner_team_id,
                    &connection.visibility,
                ) => {}
            Ok(_) => issues.push(ValidationIssue::new(
                "connectionIds",
                format!("connection '{connection_id}' is not visible to you"),
            )),
            Err(_) => issues.push(ValidationIssue::new(
                "connectionIds",
                format!("unknown connection '{connection_id}'"),
            )),
        }
    }
    for mapping_id in &body.env_mapping_ids {
        if state
            .store
            .get_env_mapping(auth.org_id, *mapping_id)
            .await
            .is_err()
        {
            issues.push(ValidationIssue::new(
                "envMappingIds",
                format!("unknown environment mapping '{mapping_id}'"),
            ));
        }
    }
    validate_native_primary(&state, auth.org_id, &body.mode, &body.steps, &mut issues).await;
    if !issues.is_empty() {
        return Err(ApiError::validation(issues));
    }
    let definition = serde_json::json!({
        "mode": body.mode,
        "instructions": body.instructions.trim(),
        "repository": body.repository.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        "branch": body.branch.as_deref().map(str::trim).filter(|value| !value.is_empty()),
        "workSource": body.work_source.as_ref().map(work_source_to_json),
        "repositories": repositories_to_json(&body.repositories),
        "model": body.model,
        "workerProfileId": body.worker_profile_id,
        "autoLaunch": body.auto_launch,
        "skills": body.skills,
        "harness": body.harness,
        "connectionIds": body.connection_ids,
        "envMappingIds": body.env_mapping_ids,
        "timeoutSeconds": body.timeout_seconds,
        "steps": body.steps.iter().map(|step| serde_json::json!({
            "agentId": step.agent_id,
            "model": step.model,
        })).collect::<Vec<_>>()
    });
    Ok(Json(
        state
            .store
            .update_agent_team(
                auth.org_id,
                team_id,
                body.name.trim(),
                body.description.trim(),
                &definition,
            )
            .await?,
    ))
}

#[utoipa::path(get, path = "/api/v1/agent-teams", tag = "agent-teams", responses((status = 200, body = [orbit_store::AgentTeam])))]
pub async fn list_teams(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<AgentTeam>>, ApiError> {
    auth.require(Permission::HarnessesRead)?;
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    Ok(Json(
        state
            .store
            .list_agent_teams(auth.org_id, Some(&access.store_access()))
            .await?,
    ))
}

#[utoipa::path(get, path = "/api/v1/agent-teams/{team_id}", tag = "agent-teams", responses((status = 200, body = orbit_store::AgentTeam)))]
pub async fn get_team(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(team_id): Path<Uuid>,
) -> Result<Json<AgentTeam>, ApiError> {
    auth.require(Permission::HarnessesRead)?;
    Ok(Json(require_team_view(&state, &auth, team_id).await?))
}

#[utoipa::path(delete, path = "/api/v1/agent-teams/{team_id}", tag = "agent-teams", responses((status = 204)))]
pub async fn delete_team(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(team_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    require_team_manage(&state, &auth, team_id).await?;
    state.store.delete_agent_team(auth.org_id, team_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn native_subagents(
    state: &AppState,
    org_id: Uuid,
    definition: &TeamDefinition,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let mut subagents = Vec::new();
    for step in definition.steps.iter().skip(1) {
        let agent = state.store.get_agent(org_id, step.agent_id).await?;
        let version = state.store.get_current_version(org_id, &agent).await?;
        let spec: orbit_domain::AgentSpec =
            serde_json::from_value(version.spec.clone()).map_err(|error| {
                ApiError::internal(format!("stored agent spec is invalid: {error}"))
            })?;
        let mut instructions = spec.instructions.system.clone();
        if !spec.instructions.constraints.is_empty() {
            instructions.push_str("\n\nConstraints:\n");
            for constraint in &spec.instructions.constraints {
                instructions.push_str(&format!("- {constraint}\n"));
            }
        }
        let model = match (&spec.model.provider, &spec.model.model) {
            (Some(provider), Some(model)) if !model.contains('/') => {
                Some(format!("{provider}/{model}"))
            }
            (_, Some(model)) => Some(model.clone()),
            _ => None,
        };
        subagents.push(serde_json::json!({
            "agentId": agent.id,
            "name": agent.name,
            "slug": agent.slug,
            "description": agent.description,
            "instructions": instructions,
            "model": model,
        }));
    }
    Ok(subagents)
}

pub(crate) async fn start_step(
    state: &AppState,
    run: &TeamRun,
    definition: &TeamDefinition,
    step_index: usize,
    previous_output: Option<&serde_json::Value>,
) -> Result<TeamStep, ApiError> {
    let step = definition
        .steps
        .get(step_index)
        .ok_or_else(|| ApiError::internal("team step index out of range"))?;
    let agent = state.store.get_agent(run.org_id, step.agent_id).await?;
    let version = state.store.get_current_version(run.org_id, &agent).await?;
    let mut spec: orbit_domain::AgentSpec = serde_json::from_value(version.spec.clone())
        .map_err(|error| ApiError::internal(format!("stored agent spec is invalid: {error}")))?;

    // Apply model override from team or step definition.
    let step_def = definition.steps.get(step_index);
    let model_override = step_def
        .and_then(|s| s.model.clone())
        .or_else(|| definition.model.clone());
    if let Some(override_model) = model_override {
        if let Some(provider) = override_model.provider {
            spec.model.provider = Some(provider);
        }
        if let Some(model) = override_model.model {
            spec.model.model = Some(model);
        }
        if let Some(management) = override_model.management {
            spec.model.management = match management.as_str() {
                "platform" => orbit_domain::ModelManagement::Platform,
                _ => orbit_domain::ModelManagement::Harness,
            };
        }
    }
    let task = run
        .input
        .get("task")
        .and_then(|value| value.as_str())
        .unwrap_or("Team run");
    let instructions = run
        .input
        .get("instructions")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let previous = previous_output
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".into());
    let task_text = if previous_output.is_none() {
        if instructions.trim().is_empty() {
            task.to_string()
        } else {
            format!("{task}\n\nTeam context: {instructions}")
        }
    } else {
        if instructions.trim().is_empty() {
            format!(
                "Role: {}\n\nOriginal task: {task}\n\nPrevious step output:\n{previous}",
                agent.name
            )
        } else {
            format!(
                "Role: {}\n\nOriginal task: {task}\n\nTeam context: {instructions}\n\nPrevious step output:\n{previous}",
                agent.name
            )
        }
    };
    let task_text = if definition.repositories.len() > 1 {
        let manifest = definition
            .repositories
            .iter()
            .map(|repository| {
                format!(
                    "- {} (workspace path: {})",
                    repository.repository,
                    if repository.path.is_empty() {
                        "."
                    } else {
                        &repository.path
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("{task_text}\n\nWorkspace repositories:\n{manifest}")
    } else {
        task_text
    };
    let requested_harness = run
        .input
        .get("payload")
        .and_then(|value| value.get("harnessOverride"))
        .and_then(|value| value.as_str())
        .or(definition.harness.as_deref());
    let harness_kind = match requested_harness {
        Some(value) => value.parse::<HarnessKind>().map_err(|_| {
            ApiError::validation(vec![ValidationIssue::new("harness", "unknown harness")])
        })?,
        None => match spec.harness.as_ref().map(|harness| harness.kind) {
            Some(kind) => kind,
            None => state.default_harness().await.ok_or_else(|| {
                ApiError::validation(vec![ValidationIssue::new(
                    "harness",
                    "no team harness, legacy agent harness or default harness is configured",
                )])
            })?,
        },
    };
    // Shared execution preflight (#140), re-evaluated for every step with the identity
    // that requested the run (falling back to the team owner).
    let team = state.store.get_agent_team(run.org_id, run.team_id).await?;
    let team_connections = team_connection_ids(definition);
    match crate::preflight::background_access(
        state,
        run.org_id,
        &[run.requested_by, team.owner_user_id],
    )
    .await?
    {
        Some(access) => {
            access.require_use(
                "agent team",
                team.owner_user_id,
                team.owner_team_id,
                &team.visibility,
            )?;
            crate::preflight::Preflight {
                state,
                org_id: run.org_id,
                access: &access,
                agent: &agent,
                spec: &spec,
                harness: harness_kind,
                extra_connection_ids: &team_connections,
            }
            .run()
            .await?;
        }
        None => {
            crate::preflight::require_harness_available(state, run.org_id, harness_kind).await?;
            crate::preflight::require_hourly_quota(state, run.org_id, agent.id, &spec).await?;
        }
    }
    let native = definition.mode == "native";
    if native && harness_kind != HarnessKind::OpenCode {
        return Err(ApiError::conflict(
            "native Agent Teams currently require an OpenCode primary agent",
        ));
    }
    let mut team_payload = serde_json::json!({
        "runId": run.id,
        "mode": definition.mode,
        "goal": instructions,
        "agent": agent.name,
        "previous": previous,
        "connectionIds": definition.connection_ids,
        "envMappingIds": definition.env_mapping_ids,
    });
    if native {
        team_payload["subagents"] =
            serde_json::Value::Array(native_subagents(state, run.org_id, definition).await?);
    }
    let input = serde_json::json!({
        "task": task_text,
        "payload": {
            "teamRunId": run.id,
            "stepIndex": step_index,
            "agentName": agent.name,
            "previous": previous_output,
            "repository": run.input.get("payload").and_then(|value| value.get("repository")).cloned(),
            "branch": run.input.get("payload").and_then(|value| value.get("branch")).cloned(),
            "repositories": run.input.get("payload").and_then(|value| value.get("repositories")).cloned(),
            "harnessOverride": run.input.get("payload").and_then(|value| value.get("harnessOverride")).cloned(),
        },
        "provenance": {
            "source": "team",
            "trusted": true,
            "subject": run.id,
            "receivedAt": chrono::Utc::now(),
        },
        "team": team_payload,
        "autonomyOverride": if run.input.get("autoApprove").and_then(|value| value.as_bool()) == Some(true) { 4 } else { 0 }
    });
    let execution = state
        .store
        .create_execution(orbit_store::executions::NewExecution {
            org_id: run.org_id,
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
            session_ref: None,
            skills: &definition.skills,
            timeout_seconds: definition
                .timeout_seconds
                .unwrap_or_else(|| spec.resources.timeout_seconds.max(3600)),
            requested_by: run.requested_by,
            idempotency_key: Some(&format!("team:{}:{}", run.id, step_index)),
            team_run_id: Some(run.id),
            team_step_index: Some(step_index as i32),
        })
        .await?;
    let created = state
        .store
        .create_team_step(
            run.id,
            run.org_id,
            step_index as i32,
            &agent.name,
            agent.id,
            version.id,
            &definition.skills,
            &execution.input,
            Some(execution.id),
        )
        .await?;
    Ok(created)
}

#[utoipa::path(post, path = "/api/v1/agent-teams/{team_id}/runs", tag = "agent-teams", request_body = CreateTeamRunRequest, responses((status = 201, body = TeamRunDetail)))]
pub async fn create_run(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(team_id): Path<Uuid>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<CreateTeamRunRequest>,
) -> Result<(StatusCode, Json<TeamRunDetail>), ApiError> {
    auth.require(Permission::ExecutionsCreate)?;
    let since = chrono::Utc::now() - chrono::Duration::hours(24);
    state.enforce_limit(
        "executions_per_day",
        state
            .store
            .count_all_executions_since(auth.org_id, since)
            .await?,
    )?;
    // Running a team needs use (not just view) on the team, its step agents' connections
    // and providers, and the team's shared connections (#140).
    let team = state.store.get_agent_team(auth.org_id, team_id).await?;
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    access.require_view(
        "agent team",
        team.owner_user_id,
        team.owner_team_id,
        &team.visibility,
    )?;
    let definition = parse_definition(&team)?;
    let step_agent_ids: Vec<Uuid> = definition.steps.iter().map(|step| step.agent_id).collect();
    crate::preflight::authorize_team_run(
        &state,
        auth.org_id,
        &access,
        &team,
        &step_agent_ids,
        &team_connection_ids(&definition),
    )
    .await?;
    validate_requested_repositories(&definition, &body.payload)?;
    let task = if body.task.trim().is_empty() {
        definition.instructions.trim()
    } else {
        body.task.trim()
    };
    if task.is_empty() {
        return Err(ApiError::validation(vec![ValidationIssue::new(
            "task",
            "task or team instructions are required",
        )]));
    }
    let mut payload = body.payload.clone();
    if !payload.is_object() {
        payload = serde_json::json!({});
    }
    if payload
        .get("repository")
        .and_then(|value| value.as_str())
        .is_none()
    {
        if let Some(repository) = &definition.repository {
            payload["repository"] = serde_json::json!(repository);
        }
    }
    if payload
        .get("branch")
        .and_then(|value| value.as_str())
        .is_none()
    {
        if let Some(branch) = &definition.branch {
            payload["branch"] = serde_json::json!(branch);
        }
    }
    if !definition.repositories.is_empty() && payload.get("repositories").is_none() {
        payload["repositories"] = serde_json::json!(definition.repositories);
    }
    if let Some(harness_override) = &body.harness_override {
        payload["harnessOverride"] = serde_json::json!(harness_override);
    }
    let input = serde_json::json!({
        "task": task,
        "instructions": definition.instructions,
        "payload": payload,
        "definition": definition,
        "autoApprove": body.auto_approve,
    });
    let run = state
        .store
        .create_team_run(auth.org_id, team.id, &input, Some(auth.user_id))
        .await?;
    let step = match start_step(&state, &run, &definition, 0, None).await {
        Ok(step) => step,
        Err(error) => {
            let _ = state
                .store
                .update_team_run(run.id, "failed", None, Some(&error.message))
                .await;
            return Err(error);
        }
    };
    if definition.auto_launch {
        if let (Some(profile_id), Some(execution_id)) =
            (definition.worker_profile_id, step.execution_id)
        {
            crate::handlers::worker_launches::launch_profile_internal(
                &state,
                &auth,
                profile_id,
                Some(execution_id),
            )
            .await?;
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(TeamRunDetail {
            run,
            steps: vec![step],
        }),
    ))
}

#[utoipa::path(get, path = "/api/v1/team-runs", tag = "agent-teams", params(ArchiveListQuery), responses((status = 200, body = [orbit_store::TeamRun])))]
pub async fn list_runs(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<ArchiveListQuery>,
) -> Result<Json<Vec<TeamRun>>, ApiError> {
    auth.require(Permission::HarnessesRead)?;
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    Ok(Json(
        state
            .store
            .list_team_runs(auth.org_id, query.archived, Some(&access.store_access()))
            .await?,
    ))
}

#[utoipa::path(get, path = "/api/v1/team-runs/{run_id}", tag = "agent-teams", responses((status = 200, body = TeamRunDetail)))]
pub async fn get_run(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<TeamRunDetail>, ApiError> {
    auth.require(Permission::HarnessesRead)?;
    let run = state.store.get_team_run(auth.org_id, run_id).await?;
    require_team_view(&state, &auth, run.team_id).await?;
    let steps = state.store.list_team_steps(run.id).await?;
    Ok(Json(TeamRunDetail { run, steps }))
}

/// Advance a running team after its current execution reaches a terminal state.
pub(crate) async fn advance(state: &AppState, run: TeamRun) -> anyhow::Result<()> {
    let Some(current) = state.store.current_team_step(run.id).await? else {
        return Ok(());
    };
    if current.status != "running" {
        return Ok(());
    }
    let Some(execution_id) = current.execution_id else {
        return Ok(());
    };
    let execution = state.store.get_execution_by_id(execution_id).await?;
    if !matches!(
        orbit_domain::ExecutionStatus::parse(&execution.status),
        Some(status) if status.is_terminal()
    ) {
        return Ok(());
    }
    let definition = match run.input.get("definition").cloned() {
        Some(value) => serde_json::from_value::<TeamDefinition>(value)
            .map_err(|error| anyhow::anyhow!("team run definition is invalid: {error}"))?,
        None => {
            let team = state.store.get_agent_team(run.org_id, run.team_id).await?;
            parse_definition(&team).map_err(|error| anyhow::anyhow!(error.message))?
        }
    };
    if execution.status == "succeeded" {
        state
            .store
            .finish_team_step(current.id, "succeeded", execution.output.as_ref(), None)
            .await?;
        if definition.mode == "native" {
            state
                .store
                .update_team_run(run.id, "succeeded", execution.output.as_ref(), None)
                .await?;
            crate::team_outcomes::close_queue_loop(
                state,
                &run,
                "succeeded",
                execution.output.as_ref(),
                None,
            )
            .await?;
            return Ok(());
        }
        let next = (current.step_index as usize) + 1;
        if next < definition.steps.len() {
            let run_for_step = state.store.get_team_run(run.org_id, run.id).await?;
            let created = match start_step(
                state,
                &run_for_step,
                &definition,
                next,
                execution.output.as_ref(),
            )
            .await
            {
                Ok(created) => created,
                // Refused by the preflight (access lost, harness/quota): fail the run
                // instead of retrying the same step every poll.
                Err(error) if error.status.is_client_error() => {
                    return crate::preflight::fail_team_run(state, &run, &error).await;
                }
                Err(error) => return Err(anyhow::anyhow!(error.message)),
            };
            if definition.auto_launch {
                if let (Some(profile_id), Some(execution_id)) =
                    (definition.worker_profile_id, created.execution_id)
                {
                    let auth = AuthContext {
                        user_id: run.requested_by.unwrap_or(Uuid::nil()),
                        org_id: run.org_id,
                        email: String::new(),
                        display_name: "Agent Team".into(),
                        role: orbit_domain::UserRole::Operator,
                        method: orbit_security::AuthMethod::ApiToken,
                        scopes: vec![],
                        token_id: None,
                    };
                    crate::handlers::worker_launches::launch_profile_internal(
                        state,
                        &auth,
                        profile_id,
                        Some(execution_id),
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!(error.message))?;
                }
            }
        } else {
            state
                .store
                .update_team_run(run.id, "succeeded", execution.output.as_ref(), None)
                .await?;
            crate::team_outcomes::close_queue_loop(
                state,
                &run,
                "succeeded",
                execution.output.as_ref(),
                None,
            )
            .await?;
        }
    } else {
        state
            .store
            .finish_team_step(
                current.id,
                "failed",
                execution.output.as_ref(),
                Some(&execution.status),
            )
            .await?;
        state
            .store
            .update_team_run(
                run.id,
                "failed",
                execution.output.as_ref(),
                Some(&format!(
                    "step '{}' finished with {}",
                    current.name, execution.status
                )),
            )
            .await?;
        crate::team_outcomes::close_queue_loop(
            state,
            &run,
            "failed",
            execution.output.as_ref(),
            Some(&format!(
                "step '{}' finished with {}",
                current.name, execution.status
            )),
        )
        .await?;
    }
    Ok(())
}
