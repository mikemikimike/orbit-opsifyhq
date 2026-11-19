use axum::extract::{Request, State};
use axum::http::header::AUTHORIZATION;
use axum::middleware::Next;
use axum::response::Response;

use orbit_domain::UserRole;
use orbit_security::{bearer_token, hash_api_token, highest_mapped_role, AuthContext, AuthMethod};

use crate::error::ApiError;
use crate::state::AppState;

/// Authenticate the request and insert an [`AuthContext`] into extensions.
pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let token = bearer_token(header);
    let context = resolve(&state, token).await?;
    request.extensions_mut().insert(context);
    Ok(next.run(request).await)
}

async fn resolve(state: &AppState, token: Option<&str>) -> Result<AuthContext, ApiError> {
    // 1) ORBIT API tokens (works in every auth mode).
    if let Some(token) = token {
        if state.auth.allow_api_tokens && token.starts_with("orbit_sk_") {
            return resolve_api_token(state, token).await;
        }
    }

    match state.auth.mode {
        orbit_security::AuthMode::Dev => resolve_dev(state, token),
        orbit_security::AuthMode::Oidc => resolve_oidc(state, token).await,
    }
}

fn resolve_dev(state: &AppState, token: Option<&str>) -> Result<AuthContext, ApiError> {
    let dev = state
        .dev_identity
        .as_ref()
        .ok_or_else(|| ApiError::internal("dev identity was not initialized"))?;
    let dev_token = token == Some(state.auth.dev.dev_token.as_str());
    let anonymous = state.auth.dev.allow_anonymous;
    // In dev mode a presented token that is not the dev token is still accepted when
    // anonymous access is enabled, so the web app works without token plumbing.
    if !dev_token && !anonymous && token.is_none() {
        return Err(ApiError::unauthorized("authentication required"));
    }
    Ok(AuthContext {
        user_id: dev.user_id,
        org_id: dev.org_id,
        email: dev.email.clone(),
        display_name: dev.display_name.clone(),
        role: dev.role,
        method: if token.is_some() {
            AuthMethod::Dev
        } else {
            AuthMethod::Anonymous
        },
        scopes: vec![],
        token_id: None,
    })
}

async fn resolve_api_token(state: &AppState, token: &str) -> Result<AuthContext, ApiError> {
    let hash = hash_api_token(token);
    let record = state
        .store
        .find_api_token_by_hash(&hash)
        .await?
        .ok_or_else(|| ApiError::unauthorized("invalid API token"))?;
    if let Some(expires_at) = record.expires_at {
        if expires_at < chrono::Utc::now() {
            return Err(ApiError::unauthorized("API token has expired"));
        }
    }
    let user = state.store.get_user(record.org_id, record.user_id).await?;
    if user.status != "active" {
        return Err(ApiError::forbidden("user is disabled"));
    }
    let (role, role_scopes) = resolve_role(state, user.org_id, &user.role).await?;
    // Token scopes are bounded by the owner's CURRENT role so demotion takes effect
    // immediately. An empty stored list keeps meaning "whatever the role grants".
    let scopes = if record.scopes.is_empty() {
        role_scopes
    } else {
        let held: Vec<String> = if role_scopes.is_empty() {
            orbit_security::Permission::all_for(role)
                .into_iter()
                .map(str::to_string)
                .collect()
        } else {
            role_scopes
        };
        let bounded = orbit_security::intersect_scopes(&record.scopes, &held);
        if bounded.is_empty() {
            // An empty list would fall back to the full role set, so deny explicitly.
            return Err(ApiError::forbidden(
                "API token has no permissions under the owner's current role",
            ));
        }
        bounded
    };
    let store = state.store.clone();
    let token_id = record.id;
    tokio::spawn(async move {
        let _ = store.touch_api_token(token_id).await;
    });
    Ok(AuthContext {
        user_id: user.id,
        org_id: user.org_id,
        email: user.email,
        display_name: user.display_name,
        role,
        method: AuthMethod::ApiToken,
        scopes,
        token_id: Some(record.id),
    })
}

async fn resolve_oidc(state: &AppState, token: Option<&str>) -> Result<AuthContext, ApiError> {
    let validator = state
        .jwt
        .as_ref()
        .ok_or_else(|| ApiError::internal("OIDC validator is not configured"))?;
    let token = token.ok_or_else(|| ApiError::unauthorized("bearer token required"))?;
    let identity = validator.validate(token).await?;
    let oidc = state
        .auth
        .oidc
        .as_ref()
        .ok_or_else(|| ApiError::internal("OIDC config missing"))?;
    // Only mappings that are explicitly configured can grant a role. With no match we
    // fall back to the configured default role.
    let mapped_role = highest_mapped_role(&identity.groups, &oidc.group_roles);
    let mapped_team = identity.groups.iter().any(|group| {
        oidc.team_groups
            .iter()
            .any(|(mapped, _, _)| mapped == group)
    });
    if oidc.require_mapped_group && mapped_role.is_none() && !mapped_team {
        return Err(ApiError::forbidden(
            "your OIDC account is not authorized for this ORBIT installation",
        ));
    }
    let target_role = mapped_role.unwrap_or(oidc.default_role);

    let user = match state
        .store
        .get_user_by_oidc(&identity.issuer, &identity.subject)
        .await?
    {
        Some(user) => {
            // Sync only when the IdP actually returned role values; a token without the
            // claim must not silently downgrade an existing user.
            if oidc.sync_roles && !identity.groups.is_empty() && user.role != target_role.as_str() {
                tracing::info!(
                    user_id = %user.id,
                    from = %user.role,
                    to = %target_role.as_str(),
                    "syncing OIDC role from group mapping"
                );
                state
                    .store
                    .set_user_role(user.org_id, user.id, target_role)
                    .await?
            } else {
                user
            }
        }
        None => {
            if !oidc.auto_provision {
                return Err(ApiError::forbidden(
                    "your identity is not provisioned in this ORBIT instance",
                ));
            }
            let seats = state.store.count_active_users(state.default_org_id).await?;
            state.enforce_limit("seats", seats)?;
            state
                .store
                .upsert_oidc_user(
                    state.default_org_id,
                    &identity.issuer,
                    &identity.subject,
                    identity.email.as_deref(),
                    identity.name.as_deref(),
                    target_role,
                )
                .await?
        }
    };
    if user.status != "active" {
        return Err(ApiError::forbidden("user is disabled"));
    }
    if !identity.groups.is_empty() && !oidc.team_groups.is_empty() {
        if let Err(error) = sync_oidc_teams(state, user.id, user.org_id, &identity.groups).await {
            tracing::warn!(%error, "OIDC access-team sync failed");
        }
    }
    let (role, scopes) = resolve_role(state, user.org_id, &user.role).await?;
    Ok(AuthContext {
        user_id: user.id,
        org_id: user.org_id,
        email: user.email,
        display_name: user.display_name,
        role,
        method: AuthMethod::Oidc,
        scopes,
        token_id: None,
    })
}

/// Additive OIDC group -> access-team membership sync. Never removes manual memberships.
async fn sync_oidc_teams(
    state: &AppState,
    user_id: uuid::Uuid,
    org_id: uuid::Uuid,
    groups: &[String],
) -> Result<(), ApiError> {
    if !state
        .license_status()
        .has_feature(orbit_security::FEATURE_TEAMS)
    {
        return Ok(());
    }
    let Some(oidc) = state.auth.oidc.as_ref() else {
        return Ok(());
    };
    let teams = state.store.list_access_teams(org_id).await?;
    for (group, team_slug, team_role) in &oidc.team_groups {
        if !groups.iter().any(|value| value == group) {
            continue;
        }
        let team = match teams.iter().find(|team| &team.slug == team_slug) {
            Some(team) => team.clone(),
            None if oidc.auto_create_teams && orbit_domain::is_valid_slug(team_slug) => match state
                .store
                .create_access_team(org_id, group, team_slug, None)
                .await
            {
                Ok(team) => {
                    tracing::info!(%group, team = %team_slug, "created access team from OIDC group");
                    team
                }
                Err(error) => {
                    tracing::warn!(%error, %group, team = %team_slug, "could not create OIDC access team");
                    continue;
                }
            },
            None => {
                tracing::warn!(%group, team = %team_slug, "OIDC team mapping references a missing team");
                continue;
            }
        };
        state
            .store
            .upsert_team_member(team.id, user_id, team_role)
            .await?;
    }
    Ok(())
}

/// Resolve a stored role name to a built-in role or a custom permission set.
async fn resolve_role(
    state: &AppState,
    org_id: uuid::Uuid,
    role_name: &str,
) -> Result<(UserRole, Vec<String>), ApiError> {
    if let Ok(role) = role_name.parse::<UserRole>() {
        return Ok((role, Vec::new()));
    }
    match state
        .store
        .custom_role_permissions(org_id, role_name)
        .await?
    {
        Some(permissions) => Ok((UserRole::Viewer, permissions)),
        None => Err(ApiError::forbidden(format!(
            "user role '{role_name}' no longer exists"
        ))),
    }
}

/// Helper for tests and for building the dev identity during bootstrap.
pub fn dev_context(state: &AppState) -> Option<AuthContext> {
    state.dev_identity.as_ref().map(|dev| AuthContext {
        user_id: dev.user_id,
        org_id: dev.org_id,
        email: dev.email.clone(),
        display_name: dev.display_name.clone(),
        role: dev.role,
        method: AuthMethod::Dev,
        scopes: vec![],
        token_id: None,
    })
}

/// Expose OIDC metadata to the web app without leaking secrets.
pub fn oidc_public_config(state: &AppState) -> serde_json::Value {
    match &state.auth.oidc {
        Some(oidc) => serde_json::json!({
            "mode": "oidc",
            "issuer": oidc.issuer,
            "clientId": oidc.client_id,
            "redirectUri": oidc.redirect_uri,
            "scopes": oidc.scopes,
        }),
        None => serde_json::json!({
            "mode": "dev",
            "devUser": state.auth.dev.email,
            "anonymousAllowed": state.auth.dev.allow_anonymous,
        }),
    }
}
