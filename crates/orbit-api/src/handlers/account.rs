use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Extension;
use axum::Json;
use chrono::{Duration, Utc};
use uuid::Uuid;

use orbit_domain::UserRole;
use orbit_security::{issue_api_token, AuthContext, Permission};
use orbit_store::{ApiTokenView, Organization, User};

use crate::audit;
use crate::dto::{AuditListQuery, CreateApiTokenRequest, CreatedApiToken, MeResponse};
use crate::error::ApiError;
use crate::handlers::limit;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/v1/me", tag = "account", responses((status = 200, body = MeResponse)))]
pub async fn me(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<MeResponse>, ApiError> {
    let user = state.store.get_user(auth.org_id, auth.user_id).await?;
    let organization = state.store.get_organization(auth.org_id).await?;
    Ok(Json(MeResponse {
        permissions: auth.permissions(),
        auth_method: format!("{:?}", auth.method).to_lowercase(),
        scopes: auth.scopes.clone(),
        user,
        organization,
    }))
}

#[utoipa::path(get, path = "/api/v1/organizations/current", tag = "account", responses((status = 200, body = Organization)))]
pub async fn current_organization(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Organization>, ApiError> {
    auth.require(Permission::MembersRead)?;
    Ok(Json(state.store.get_organization(auth.org_id).await?))
}

#[utoipa::path(get, path = "/api/v1/users", tag = "account", responses((status = 200, body = [User])))]
pub async fn list_users(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<User>>, ApiError> {
    auth.require(Permission::MembersRead)?;
    Ok(Json(state.store.list_users(auth.org_id).await?))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRoleRequest {
    pub role: String,
}

#[utoipa::path(patch, path = "/api/v1/users/{user_id}", tag = "account", request_body = UpdateUserRoleRequest, responses((status = 200, body = User)))]
pub async fn update_user_role(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(user_id): Path<Uuid>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<UpdateUserRoleRequest>,
) -> Result<Json<User>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let role_name = body.role.trim();
    let builtin = role_name.parse::<UserRole>().is_ok();
    let custom = !builtin
        && state
            .store
            .custom_role_permissions(auth.org_id, role_name)
            .await?
            .is_some();
    if !builtin && !custom {
        return Err(ApiError::bad_request("invalid role"));
    }
    if custom {
        state.require_feature(orbit_security::license::FEATURE_CUSTOM_ROLES)?;
    }
    if user_id == auth.user_id {
        return Err(ApiError::bad_request("you cannot change your own role"));
    }
    let user = state
        .store
        .set_user_role_name(auth.org_id, user_id, role_name)
        .await?;
    audit::record(
        &state,
        &auth,
        "user.role_changed",
        Some("user"),
        Some(user_id),
        serde_json::json!({ "role": body.role }),
    )
    .await;
    Ok(Json(user))
}

#[utoipa::path(get, path = "/api/v1/api-tokens", tag = "account", responses((status = 200, body = [ApiTokenView])))]
pub async fn list_api_tokens(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<Vec<ApiTokenView>>, ApiError> {
    auth.require(Permission::SettingsRead)?;
    Ok(Json(state.store.list_api_tokens(auth.org_id).await?))
}

#[utoipa::path(post, path = "/api/v1/api-tokens", tag = "account", request_body = CreateApiTokenRequest, responses((status = 201, body = CreatedApiToken)))]
pub async fn create_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<CreateApiTokenRequest>,
) -> Result<(StatusCode, Json<CreatedApiToken>), ApiError> {
    auth.require(Permission::SettingsWrite)?;
    if body.name.trim().is_empty() {
        return Err(ApiError::validation(vec![
            orbit_domain::ValidationIssue::new("name", "token name is required"),
        ]));
    }
    let scopes = if body.scopes.is_empty() {
        auth.permissions()
    } else {
        body.scopes.clone()
    };
    let valid: std::collections::BTreeSet<&str> =
        Permission::all_for(UserRole::Owner).into_iter().collect();
    for scope in &scopes {
        if scope != "*" && !valid.contains(scope.as_str()) {
            return Err(ApiError::validation(vec![
                orbit_domain::ValidationIssue::new("scopes", format!("unknown scope '{scope}'")),
            ]));
        }
    }
    // `*` means "everything my role grants" and is bounded at authentication time, so it
    // is always allowed; explicit scopes must be held by the caller right now.
    let held = auth.permissions();
    let denied: Vec<orbit_domain::ValidationIssue> = scopes
        .iter()
        .filter(|scope| scope.as_str() != "*" && !orbit_security::scope_granted(&held, scope))
        .map(|scope| {
            orbit_domain::ValidationIssue::new(
                "scopes",
                format!("scope '{scope}' exceeds your current permissions"),
            )
        })
        .collect();
    if !denied.is_empty() {
        return Err(ApiError::validation(denied));
    }
    let expires_at = body
        .expires_in_days
        .filter(|days| *days > 0)
        .map(|days| Utc::now() + Duration::days(days.min(3650)));
    let issued = issue_api_token();
    let view = state
        .store
        .create_api_token(
            auth.org_id,
            auth.user_id,
            body.name.trim(),
            &issued.prefix,
            &issued.hash,
            &scopes,
            expires_at,
        )
        .await?;
    audit::record(
        &state,
        &auth,
        "api_token.created",
        Some("api_token"),
        Some(view.id),
        serde_json::json!({ "name": view.name, "scopes": scopes }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(CreatedApiToken {
            id: view.id,
            name: view.name,
            token: issued.token,
            token_prefix: view.token_prefix,
            scopes: view.scopes,
            expires_at: view.expires_at,
        }),
    ))
}

#[utoipa::path(delete, path = "/api/v1/api-tokens/{token_id}", tag = "account", responses((status = 204)))]
pub async fn revoke_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(token_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    state.store.revoke_api_token(auth.org_id, token_id).await?;
    audit::record(
        &state,
        &auth,
        "api_token.revoked",
        Some("api_token"),
        Some(token_id),
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuditLogPage {
    pub items: Vec<orbit_store::AuditLogRecord>,
    pub next_cursor: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/audit-logs", tag = "account", params(AuditListQuery), responses((status = 200, body = AuditLogPage)))]
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<AuditListQuery>,
) -> Result<Json<AuditLogPage>, ApiError> {
    auth.require(Permission::AuditRead)?;
    let (items, next) = state
        .store
        .list_audit_logs(
            auth.org_id,
            limit(query.limit),
            query.before_id,
            query.action.as_deref(),
        )
        .await?;
    Ok(Json(AuditLogPage {
        items,
        next_cursor: next.map(|id| id.to_string()),
    }))
}
