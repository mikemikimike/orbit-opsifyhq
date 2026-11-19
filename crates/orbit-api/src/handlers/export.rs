//! Secret-free resource export for sharing outside this installation.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Extension;
use uuid::Uuid;

use orbit_security::{AuthContext, Permission};

use crate::error::ApiError;
use crate::state::AppState;

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "token",
        "secret",
        "password",
        "credential",
        "apikey",
        "authorization",
        "privatekey",
        "clientsecret",
        "webhooksecret",
        "authsecretname",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn redact(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|key, _| !sensitive_key(key));
            for value in map.values_mut() {
                redact(value);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact(item);
            }
        }
        serde_json::Value::String(text)
            if text.starts_with("orbit_sk_") || text.starts_with("orbit_wk_") =>
        {
            *text = "[redacted]".into();
        }
        _ => {}
    }
}

/// The read permission that guards listing the resource kind (#140).
fn read_permission(resource: &str) -> Permission {
    match resource {
        "providers" => Permission::ProvidersRead,
        "connections" => Permission::ConnectionsRead,
        "worker_profiles" | "agent_teams" => Permission::HarnessesRead,
        "tool_policies" => Permission::SettingsRead,
        _ => Permission::AgentsRead,
    }
}

#[utoipa::path(get, path = "/api/v1/export/{resource}/{resource_id}", tag = "export", responses((status = 200, description = "Secret-free resource export")))]
pub async fn export_resource(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((resource, resource_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(read_permission(&resource))?;
    let (owner_user, owner_team, visibility) = state
        .store
        .resource_ownership(auth.org_id, &resource, resource_id)
        .await?;
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    access.require_view(&resource, owner_user, owner_team, &visibility)?;

    let data = match resource.as_str() {
        "agents" => {
            let agent = state.store.get_agent(auth.org_id, resource_id).await?;
            let version = state.store.get_current_version(auth.org_id, &agent).await?;
            let spec: serde_json::Value = version.spec;
            serde_json::json!({
                "apiVersion": orbit_domain::AGENT_API_VERSION,
                "kind": orbit_domain::AGENT_KIND,
                "metadata": {
                    "name": agent.name,
                    "description": agent.description,
                    "labels": agent.labels,
                },
                "spec": spec,
            })
        }
        "skills" => serde_json::to_value(state.store.get_skill(auth.org_id, resource_id).await?)
            .map_err(|error| ApiError::internal(error.to_string()))?,
        "connections" => {
            let row = state
                .store
                .get_connection_with_connector(auth.org_id, resource_id)
                .await?;
            serde_json::json!({
                "connection": row.connection,
                "connector": row.connector_slug,
            })
        }
        "providers" => {
            serde_json::to_value(state.store.get_provider(auth.org_id, resource_id).await?)
                .map_err(|error| ApiError::internal(error.to_string()))?
        }
        "worker_profiles" => serde_json::to_value(
            state
                .store
                .get_worker_profile(auth.org_id, resource_id)
                .await?,
        )
        .map_err(|error| ApiError::internal(error.to_string()))?,
        "triggers" => {
            serde_json::to_value(state.store.get_trigger(auth.org_id, resource_id).await?)
                .map_err(|error| ApiError::internal(error.to_string()))?
        }
        "tool_policies" => serde_json::to_value(
            state
                .store
                .get_tool_policy(auth.org_id, resource_id)
                .await?,
        )
        .map_err(|error| ApiError::internal(error.to_string()))?,
        "agent_teams" => {
            serde_json::to_value(state.store.get_agent_team(auth.org_id, resource_id).await?)
                .map_err(|error| ApiError::internal(error.to_string()))?
        }
        other => return Err(ApiError::bad_request(format!("unknown resource '{other}'"))),
    };
    let mut data = data;
    redact(&mut data);
    let filename = format!(
        "{}-{}.json",
        data.pointer("/metadata/name")
            .or_else(|| data.get("name"))
            .and_then(|value| value.as_str())
            .unwrap_or(&resource),
        &resource_id.to_string()[..8]
    );
    let mut response = axum::Json(serde_json::json!({
        "resource": resource,
        "id": resource_id,
        "exportedAt": chrono::Utc::now(),
        "data": data,
    }))
    .into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse()
            .map_err(|_| ApiError::internal("invalid export filename"))?,
    );
    let _ = headers;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_fields_and_tokens() {
        let mut value = serde_json::json!({
            "name": "safe",
            "apiKey": "abc",
            "nested": { "password": "x", "tokenValue": "y", "keep": 1 },
            "list": [{ "secretRef": "z", "value": "orbit_sk_123" }]
        });
        redact(&mut value);
        assert_eq!(value["name"], "safe");
        assert!(value.get("apiKey").is_none());
        assert!(value["nested"].get("password").is_none());
        assert!(value["nested"].get("tokenValue").is_none());
        assert_eq!(value["nested"]["keep"], 1);
        assert!(value["list"][0].get("secretRef").is_none());
        assert_eq!(value["list"][0]["value"], "[redacted]");
    }
}
