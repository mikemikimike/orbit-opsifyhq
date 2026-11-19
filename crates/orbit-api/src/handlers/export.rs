//! Secret-free resource export for sharing outside this installation.
//!
//! `GET /api/v1/export/{resource}/{id}` supports three formats:
//!
//! * `json` (default, legacy): `{ resource, id, exportedAt, data }` with the stored
//!   shape of the resource, recursively redacted.
//! * `yaml`: the portable envelope (`apiVersion`/`kind`/`metadata`/`spec`) defined in
//!   `orbit_contracts::portable`. References use names, never ids.
//! * `markdown`: skills only, as `SKILL.md` (frontmatter + instructions).

use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Extension;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use orbit_contracts::portable::{
    redact, skill_to_markdown, PortableDocument, PortableKind, VALUE_PLACEHOLDER,
};
use orbit_domain::AgentDefinition;
use orbit_security::{AuthContext, Permission};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
pub struct ExportQuery {
    /// `json` (default), `yaml` (portable envelope) or `markdown` (skills: SKILL.md).
    pub format: Option<String>,
}

/// Read permission required to export a given kind.
pub(crate) fn read_permission(kind: PortableKind) -> Permission {
    match kind {
        PortableKind::Agent | PortableKind::Skill | PortableKind::Trigger => Permission::AgentsRead,
        PortableKind::Connection => Permission::ConnectionsRead,
        PortableKind::Provider => Permission::ProvidersRead,
        PortableKind::WorkerProfile | PortableKind::AgentTeam => Permission::HarnessesRead,
        PortableKind::ToolPolicy => Permission::SettingsRead,
    }
}

fn internal<E: std::fmt::Display>(error: E) -> ApiError {
    ApiError::internal(error.to_string())
}

fn redacted(mut value: Value) -> Value {
    redact(&mut value);
    value
}

fn string_map(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// `<connector>/<name>` reference for a connection id, when it still exists.
async fn connection_ref(state: &AppState, org_id: Uuid, id: Uuid) -> Option<String> {
    let row = state
        .store
        .get_connection_with_connector(org_id, id)
        .await
        .ok()?;
    Some(format!("{}/{}", row.connector_slug, row.connection.name))
}

async fn agent_name(state: &AppState, org_id: Uuid, id: Uuid) -> Option<String> {
    state
        .store
        .get_agent(org_id, id)
        .await
        .ok()
        .map(|agent| agent.name)
}

/// Replace `connectionId` with a portable `connection: <connector>/<name>` in a JSON
/// object (work source, repository entry). Unknown ids are dropped.
async fn portable_connection_field(state: &AppState, org_id: Uuid, value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let Some(id) = object.remove("connectionId") else {
        return;
    };
    if let Some(id) = id.as_str().and_then(|id| id.parse::<Uuid>().ok()) {
        if let Some(reference) = connection_ref(state, org_id, id).await {
            object.insert("connection".into(), Value::String(reference));
        }
    }
}

/// Build the portable document for a stored resource. Returns the document and the
/// raw name used for file naming.
pub(crate) async fn build_document(
    state: &AppState,
    org_id: Uuid,
    kind: PortableKind,
    id: Uuid,
) -> Result<PortableDocument, ApiError> {
    let store = &state.store;
    let document = match kind {
        PortableKind::Agent => {
            let agent = store.get_agent(org_id, id).await?;
            let version = store.get_current_version(org_id, &agent).await?;
            let spec: orbit_domain::AgentSpec = super::parse_spec(&version)?;
            let definition = AgentDefinition {
                api_version: orbit_domain::AGENT_API_VERSION.to_string(),
                kind: orbit_domain::AGENT_KIND.to_string(),
                metadata: orbit_domain::AgentMetadata {
                    name: agent.name.clone(),
                    description: agent.description.clone(),
                    labels: serde_json::from_value(agent.labels.clone()).unwrap_or_default(),
                },
                spec,
            };
            let mut spec = serde_json::to_value(&definition.spec).map_err(internal)?;
            // Connection ids are installation-specific: export `connection:
            // <connector>/<name>` instead. Everything credential-shaped (webhook
            // `secretRef`, harness config secrets) is removed by `redact`.
            if let Some(bindings) = spec.get_mut("connectors").and_then(Value::as_array_mut) {
                for binding in bindings {
                    if let Some(object) = binding.as_object_mut() {
                        if let Some(connection_id) = object
                            .remove("connectionId")
                            .and_then(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()))
                        {
                            if let Some(reference) =
                                connection_ref(state, org_id, connection_id).await
                            {
                                object.insert("connection".into(), Value::String(reference));
                            }
                        }
                    }
                }
            }
            let mut doc = PortableDocument::new(PortableKind::Agent, agent.name, redacted(spec));
            doc.metadata.description = agent.description;
            doc.metadata.labels = definition.metadata.labels;
            doc
        }
        PortableKind::Skill => {
            let skill = store.get_skill(org_id, id).await?;
            let mut doc = PortableDocument::new(
                PortableKind::Skill,
                skill.name.clone(),
                json!({
                    "slug": skill.slug,
                    "content": skill.content,
                    "files": string_map(&skill.files),
                }),
            );
            if doc.spec["files"].as_object().is_some_and(|m| m.is_empty()) {
                if let Some(object) = doc.spec.as_object_mut() {
                    object.remove("files");
                }
            }
            doc.metadata.description = skill.description;
            doc
        }
        PortableKind::Connection => {
            let row = store.get_connection_with_connector(org_id, id).await?;
            let has_credential = row.connection.credential_ref.is_some();
            let mut doc = PortableDocument::new(
                PortableKind::Connection,
                row.connection.name.clone(),
                json!({
                    "connector": row.connector_slug,
                    "scopes": row.connection.scopes,
                    "config": redacted(row.connection.config.clone()),
                }),
            );
            doc = doc.with_credentials(has_credential);
            doc
        }
        PortableKind::Provider => {
            let provider = store.get_provider(org_id, id).await?;
            let has_credential = provider.secret_ref.is_some();
            PortableDocument::new(
                PortableKind::Provider,
                provider.name.clone(),
                json!({
                    "kind": provider.kind,
                    "enabled": provider.enabled,
                    "config": redacted(provider.config.clone()),
                }),
            )
            .with_credentials(has_credential)
        }
        PortableKind::WorkerProfile => {
            let profile = store.get_worker_profile(org_id, id).await?;
            let mut agents = Vec::new();
            for agent_id in &profile.agent_ids {
                if let Some(name) = agent_name(state, org_id, *agent_id).await {
                    agents.push(name);
                }
            }
            let env_mappings: Vec<String> = store
                .list_worker_profile_env_mappings(profile.id)
                .await?
                .into_iter()
                .map(|mapping| mapping.name)
                .collect();
            let env: BTreeMap<String, String> = profile
                .env
                .as_object()
                .map(|map| {
                    map.keys()
                        .map(|key| (key.clone(), VALUE_PLACEHOLDER.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let needs_credentials = !env.is_empty() || profile.auth_secret_name.is_some();
            let mut spec = redacted(json!({
                "harnessKind": profile.harness_kind,
                "target": profile.target,
                "executionDriver": profile.execution_driver,
                "image": profile.image,
                "imageTag": profile.image_tag,
                "agentScope": profile.agent_scope,
                "agents": agents,
                "skills": profile.skill_slugs,
                "provider": profile.provider,
                "model": profile.model,
                "envMappings": env_mappings,
                "resources": profile.resources,
                "allowWebSearch": profile.allow_web_search,
                "allowWebFetch": profile.allow_web_fetch,
                "allowGlab": profile.allow_glab,
                "allowGh": profile.allow_gh,
                "nativeToolPermissions": profile.native_tool_permissions,
            }));
            // `env` is built after redaction: names are kept, values never leave.
            spec["env"] = serde_json::to_value(env).map_err(internal)?;
            PortableDocument::new(PortableKind::WorkerProfile, profile.name, spec)
                .with_credentials(needs_credentials)
        }
        PortableKind::AgentTeam => {
            let team = store.get_agent_team(org_id, id).await?;
            let definition = super::teams::parse_definition(&team)?;
            let worker_profile = match definition.worker_profile_id {
                Some(profile_id) => store
                    .get_worker_profile(org_id, profile_id)
                    .await
                    .ok()
                    .map(|profile| profile.name),
                None => None,
            };
            let mut connections = Vec::new();
            for connection_id in &definition.connection_ids {
                if let Some(reference) = connection_ref(state, org_id, *connection_id).await {
                    connections.push(reference);
                }
            }
            let mut env_mappings = Vec::new();
            for mapping_id in &definition.env_mapping_ids {
                if let Ok(mapping) = store.get_env_mapping(org_id, *mapping_id).await {
                    env_mappings.push(mapping.name);
                }
            }
            let mut steps = Vec::new();
            for step in &definition.steps {
                if let Some(name) = agent_name(state, org_id, step.agent_id).await {
                    let mut entry = json!({ "agent": name });
                    if let Some(model) = &step.model {
                        entry["model"] = serde_json::to_value(model).map_err(internal)?;
                    }
                    steps.push(entry);
                }
            }
            let mut work_source = match &definition.work_source {
                Some(source) => Some(serde_json::to_value(source).map_err(internal)?),
                None => None,
            };
            if let Some(source) = work_source.as_mut() {
                portable_connection_field(state, org_id, source).await;
            }
            let mut repositories = Vec::new();
            for repository in &definition.repositories {
                let mut value = serde_json::to_value(repository).map_err(internal)?;
                portable_connection_field(state, org_id, &mut value).await;
                if value.get("connectionId").is_none() {
                    if let Some(object) = value.as_object_mut() {
                        object.retain(|_, v| !v.is_null());
                    }
                }
                repositories.push(value);
            }
            let spec = redacted(json!({
                "mode": definition.mode,
                "instructions": definition.instructions,
                "repository": definition.repository,
                "branch": definition.branch,
                "model": definition.model,
                "workSource": work_source,
                "repositories": repositories,
                "workerProfile": worker_profile,
                "autoLaunch": definition.auto_launch,
                "skills": definition.skills,
                "harness": definition.harness,
                "connections": connections,
                "envMappings": env_mappings,
                "timeoutSeconds": definition.timeout_seconds,
                "steps": steps,
            }));
            let mut doc = PortableDocument::new(PortableKind::AgentTeam, team.name, spec);
            doc.metadata.description = team.description;
            doc
        }
        PortableKind::Trigger => {
            let trigger = store.get_trigger(org_id, id).await?;
            let agent = agent_name(state, org_id, trigger.agent_id)
                .await
                .ok_or_else(|| ApiError::not_found("agent of trigger"))?;
            let webhook = trigger.kind == "webhook";
            let mut doc = PortableDocument::new(
                PortableKind::Trigger,
                format!("{agent} {}", trigger.kind),
                json!({
                    "agent": agent,
                    "kind": trigger.kind,
                    "harness": trigger.harness_kind,
                    "enabled": trigger.enabled,
                    "config": redacted(trigger.config.clone()),
                }),
            );
            if webhook {
                doc = doc.with_credentials(true);
            }
            doc
        }
        PortableKind::ToolPolicy => {
            let policy = store.get_tool_policy(org_id, id).await?;
            let agent = match policy.agent_id {
                Some(agent_id) => agent_name(state, org_id, agent_id).await,
                None => None,
            };
            let mut doc = PortableDocument::new(
                PortableKind::ToolPolicy,
                policy.name.clone(),
                redacted(json!({
                    "agent": agent,
                    "priority": policy.priority,
                    "enabled": policy.enabled,
                    "action": policy.action,
                    "matcher": policy.matcher,
                    "reason": policy.reason,
                    "impact": policy.impact,
                })),
            );
            if let Some(object) = doc.spec.as_object_mut() {
                object.retain(|_, v| !v.is_null());
            }
            doc
        }
    };
    Ok(document)
}

fn file_stem(name: &str) -> String {
    let slug = orbit_contracts::portable::slugify(name);
    if slug.is_empty() {
        "export".into()
    } else {
        slug
    }
}

fn attachment(
    mut response: Response,
    content_type: &'static str,
    filename: &str,
) -> Result<Response, ApiError> {
    let safe: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{safe}\"")
            .parse()
            .map_err(|_| ApiError::internal("invalid export filename"))?,
    );
    Ok(response)
}

#[utoipa::path(get, path = "/api/v1/export/{resource}/{resource_id}", tag = "export", params(("resource" = String, Path, description = "agents, skills, connections, providers, worker_profiles, agent_teams, triggers or tool_policies"), ("resource_id" = Uuid, Path), ExportQuery), responses((status = 200, description = "Secret-free resource export (JSON, portable YAML or SKILL.md)")))]
pub async fn export_resource(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((resource, resource_id)): Path<(String, Uuid)>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, ApiError> {
    let kind = PortableKind::from_resource(&resource)
        .ok_or_else(|| ApiError::bad_request(format!("unknown resource '{resource}'")))?;
    auth.require(read_permission(kind))?;
    let (owner_user, owner_team, visibility) = state
        .store
        .resource_ownership(auth.org_id, &resource, resource_id)
        .await?;
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    access.require_view(&resource, owner_user, owner_team, &visibility)?;

    let format = query.format.as_deref().unwrap_or("json");
    let document = build_document(&state, auth.org_id, kind, resource_id).await?;
    let stem = file_stem(&document.metadata.name);
    match format {
        "yaml" | "yml" => {
            let text = document.to_yaml().map_err(internal)?;
            attachment(
                text.into_response(),
                "application/yaml; charset=utf-8",
                &format!("{stem}.{}.yaml", kind.as_str().to_ascii_lowercase()),
            )
        }
        "markdown" | "md" => {
            if kind != PortableKind::Skill {
                return Err(ApiError::bad_request(
                    "markdown export is only available for skills",
                ));
            }
            let spec = &document.spec;
            let text = skill_to_markdown(
                spec["slug"].as_str().unwrap_or_default(),
                &document.metadata.name,
                &document.metadata.description,
                spec["content"].as_str().unwrap_or_default(),
            );
            let omitted = spec
                .get("files")
                .and_then(Value::as_object)
                .map_or(0, |files| files.len());
            let mut response = attachment(
                text.into_response(),
                "text/markdown; charset=utf-8",
                "SKILL.md",
            )?;
            if omitted > 0 {
                // SKILL.md cannot carry bundled files; the YAML export does.
                response.headers_mut().insert(
                    "x-orbit-omitted-files",
                    HeaderValue::from_str(&omitted.to_string()).map_err(internal)?,
                );
            }
            Ok(response)
        }
        "json" => {
            let data = serde_json::to_value(&document).map_err(internal)?;
            let filename = format!("{stem}-{}.json", &resource_id.to_string()[..8]);
            let response = axum::Json(json!({
                "resource": resource,
                "id": resource_id,
                "exportedAt": chrono::Utc::now(),
                "data": data,
            }))
            .into_response();
            attachment(response, "application/json", &filename)
        }
        other => Err(ApiError::bad_request(format!(
            "unknown format '{other}', expected json, yaml or markdown"
        ))),
    }
}
