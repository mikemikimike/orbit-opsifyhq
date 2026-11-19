//! Platform settings (OIDC, etc.) manageable from the UI.

use axum::extract::State;
use axum::Extension;
use axum::Json;
use serde::{Deserialize, Serialize};

use orbit_security::{AuthContext, Permission};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OidcSettings {
    #[serde(default)]
    pub issuer: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub scopes: String,
    #[serde(default)]
    pub audience: String,
    #[serde(default)]
    pub jwks_url: String,
    #[serde(default)]
    pub auto_provision: bool,
    #[serde(default)]
    pub default_role: String,
    /// Claim carrying IdP groups/roles (default `groups`).
    #[serde(default)]
    pub role_claim: String,
    /// Claim used as the unique OIDC identity/email (default `email`).
    #[serde(default)]
    pub email_claim: String,
    /// IdP group/claim value -> ORBIT role name.
    #[serde(default)]
    pub group_roles: std::collections::BTreeMap<String, String>,
    /// IdP group -> `team-slug:role` (access team membership).
    #[serde(default)]
    pub team_groups: std::collections::BTreeMap<String, String>,
    /// Create missing access teams for mapped groups.
    #[serde(default)]
    pub auto_create_teams: bool,
    /// Deny users whose groups match no role/team mapping.
    #[serde(default)]
    pub require_mapped_group: bool,
    /// Re-evaluate the stored role on every login when a mapping matches.
    #[serde(default)]
    pub sync_roles: bool,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOidcRequest {
    pub settings: OidcSettings,
}

#[utoipa::path(get, path = "/api/v1/settings/oidc", tag = "settings", responses((status = 200, body = OidcSettings)))]
pub async fn get_oidc(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<OidcSettings>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let value = state
        .store
        .get_platform_setting("oidc")
        .await?
        .unwrap_or_else(|| serde_json::json!({}));
    let settings: OidcSettings = serde_json::from_value(value).unwrap_or(OidcSettings {
        issuer: String::new(),
        client_id: String::new(),
        redirect_uri: String::new(),
        scopes: "openid profile email".into(),
        audience: String::new(),
        jwks_url: String::new(),
        auto_provision: true,
        default_role: "viewer".into(),
        role_claim: "groups".into(),
        email_claim: "email".into(),
        group_roles: Default::default(),
        team_groups: Default::default(),
        auto_create_teams: false,
        require_mapped_group: false,
        sync_roles: false,
    });
    Ok(Json(settings))
}

#[utoipa::path(put, path = "/api/v1/settings/oidc", tag = "settings", request_body = UpdateOidcRequest, responses((status = 200, body = OidcSettings)))]
pub async fn update_oidc(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<UpdateOidcRequest>,
) -> Result<Json<OidcSettings>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let value =
        serde_json::to_value(&body.settings).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let stored = state
        .store
        .set_platform_setting("oidc", &value, Some(auth.user_id))
        .await?;
    let settings: OidcSettings = serde_json::from_value(stored).unwrap_or(body.settings);
    Ok(Json(settings))
}

/// Merge OIDC settings stored via the Settings UI into the environment-derived auth
/// config. Only applies in OIDC mode; environment values remain the fallback for any
/// field left empty in the UI.
pub async fn apply_stored_oidc(
    store: &orbit_store::Store,
    mut auth: orbit_security::AuthConfig,
) -> orbit_security::AuthConfig {
    if !matches!(auth.mode, orbit_security::AuthMode::Oidc) {
        return auth;
    }
    let Ok(Some(value)) = store.get_platform_setting("oidc").await else {
        return auth;
    };
    let Ok(stored) = serde_json::from_value::<OidcSettings>(value) else {
        return auth;
    };
    if stored.issuer.trim().is_empty() {
        return auth;
    }
    let mut oidc = auth.oidc.clone().unwrap_or(orbit_security::OidcConfig {
        issuer: String::new(),
        audience: String::new(),
        jwks_url: String::new(),
        client_id: String::new(),
        client_secret: None,
        redirect_uri: String::new(),
        scopes: vec!["openid".into(), "profile".into(), "email".into()],
        auto_provision: true,
        default_role: orbit_domain::UserRole::Viewer,
        role_claim: "groups".into(),
        email_claim: "email".into(),
        group_roles: Vec::new(),
        team_groups: Vec::new(),
        auto_create_teams: false,
        require_mapped_group: false,
        sync_roles: false,
        clock_skew_seconds: 60,
    });
    oidc.issuer = stored.issuer.trim().to_string();
    if !stored.audience.trim().is_empty() {
        oidc.audience = stored.audience.trim().to_string();
    }
    if oidc.audience.is_empty() {
        oidc.audience = oidc.issuer.clone();
    }
    if !stored.client_id.trim().is_empty() {
        oidc.client_id = stored.client_id.trim().to_string();
    }
    if oidc.client_id.is_empty() {
        oidc.client_id = oidc.audience.clone();
    }
    if !stored.redirect_uri.trim().is_empty() {
        oidc.redirect_uri = stored.redirect_uri.trim().to_string();
    }
    if !stored.scopes.trim().is_empty() {
        oidc.scopes = stored
            .scopes
            .split_whitespace()
            .map(str::to_string)
            .collect();
    }
    if !stored.jwks_url.trim().is_empty() {
        oidc.jwks_url = stored.jwks_url.trim().to_string();
    }
    if oidc.jwks_url.is_empty() {
        oidc.jwks_url = format!(
            "{}/.well-known/jwks.json",
            oidc.issuer.trim_end_matches('/')
        );
    }
    oidc.auto_provision = stored.auto_provision;
    oidc.default_role = stored
        .default_role
        .parse::<orbit_domain::UserRole>()
        .unwrap_or(orbit_domain::UserRole::Viewer);
    if !stored.role_claim.trim().is_empty() {
        oidc.role_claim = stored.role_claim.trim().to_string();
    }
    if !stored.email_claim.trim().is_empty() {
        oidc.email_claim = stored.email_claim.trim().to_string();
    }
    if !stored.group_roles.is_empty() {
        let mut mappings = Vec::new();
        for (group, role) in &stored.group_roles {
            match role.parse::<orbit_domain::UserRole>() {
                Ok(role) => mappings.push((group.clone(), role)),
                Err(_) => tracing::warn!(
                    group = %group,
                    role = %role,
                    "ignoring OIDC group mapping with an unknown role"
                ),
            }
        }
        oidc.group_roles = mappings;
    }
    if !stored.team_groups.is_empty() {
        let mut mappings = Vec::new();
        for (group, value) in &stored.team_groups {
            match value.split_once(':') {
                Some((team, role))
                    if !team.trim().is_empty()
                        && matches!(role.trim(), "lead" | "member" | "viewer") =>
                {
                    mappings.push((
                        group.clone(),
                        team.trim().to_string(),
                        role.trim().to_string(),
                    ));
                }
                _ => tracing::warn!(group = %group, "ignoring invalid OIDC team mapping"),
            }
        }
        oidc.team_groups = mappings;
    }
    oidc.auto_create_teams = stored.auto_create_teams;
    oidc.require_mapped_group = stored.require_mapped_group;
    if stored.sync_roles {
        oidc.sync_roles = true;
    }
    tracing::info!(issuer = %oidc.issuer, "applied OIDC settings from the database");
    auth.oidc = Some(oidc);
    auth
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccessSettings {
    /// Non-admins only see resources owned by their access teams.
    pub team_isolation: bool,
}

fn access_settings(value: serde_json::Value) -> AccessSettings {
    AccessSettings {
        team_isolation: value
            .get("enabled")
            .and_then(|enabled| enabled.as_bool())
            .unwrap_or(true),
    }
}

#[utoipa::path(get, path = "/api/v1/settings/access", tag = "settings", responses((status = 200, body = AccessSettings)))]
pub async fn get_access(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<AccessSettings>, ApiError> {
    auth.require(Permission::SettingsRead)?;
    Ok(Json(access_settings(
        state
            .store
            .get_platform_setting("team_isolation")
            .await?
            .unwrap_or_else(|| serde_json::json!({ "enabled": true })),
    )))
}

#[utoipa::path(put, path = "/api/v1/settings/access", tag = "settings", request_body = AccessSettings, responses((status = 200, body = AccessSettings)))]
pub async fn update_access(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<AccessSettings>,
) -> Result<Json<AccessSettings>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let value = serde_json::json!({ "enabled": body.team_isolation });
    state
        .store
        .set_platform_setting("team_isolation", &value, Some(auth.user_id))
        .await?;
    Ok(Json(body))
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSettings {
    /// Harness used when an execution/trigger does not select one.
    pub default_harness: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/settings/harness", tag = "settings", responses((status = 200, body = HarnessSettings)))]
pub async fn get_harness(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<HarnessSettings>, ApiError> {
    auth.require(Permission::SettingsRead)?;
    let value = state
        .store
        .get_platform_setting("harness_default")
        .await?
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(Json(HarnessSettings {
        default_harness: value
            .get("default")
            .and_then(|value| value.as_str())
            .map(str::to_string),
    }))
}

#[utoipa::path(put, path = "/api/v1/settings/harness", tag = "settings", request_body = HarnessSettings, responses((status = 200, body = HarnessSettings)))]
pub async fn update_harness(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<HarnessSettings>,
) -> Result<Json<HarnessSettings>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    if let Some(harness) = &body.default_harness {
        harness.parse::<orbit_domain::HarnessKind>().map_err(|_| {
            ApiError::validation(vec![orbit_domain::ValidationIssue::new(
                "defaultHarness",
                "unknown harness",
            )])
        })?;
    }
    let value = serde_json::json!({ "default": body.default_harness });
    state
        .store
        .set_platform_setting("harness_default", &value, Some(auth.user_id))
        .await?;
    Ok(Json(body))
}

/// Platform-setting key holding the approvals policy.
pub const APPROVALS_SETTING_KEY: &str = "approvals";

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalsSettings {
    /// Four-eyes: the user who requested an execution cannot approve or reject the
    /// approvals that execution raises. Default `false`.
    #[serde(default)]
    pub approvals_require_different_approver: bool,
}

/// Read the approvals policy; missing or malformed values fall back to the default (off).
pub async fn load_approvals_settings(
    store: &orbit_store::Store,
) -> Result<ApprovalsSettings, ApiError> {
    let value = store.get_platform_setting(APPROVALS_SETTING_KEY).await?;
    Ok(value
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or(ApprovalsSettings {
            approvals_require_different_approver: false,
        }))
}

#[utoipa::path(get, path = "/api/v1/settings/approvals", tag = "settings", responses((status = 200, body = ApprovalsSettings)))]
pub async fn get_approvals(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<ApprovalsSettings>, ApiError> {
    auth.require(Permission::SettingsRead)?;
    Ok(Json(load_approvals_settings(&state.store).await?))
}

#[utoipa::path(put, path = "/api/v1/settings/approvals", tag = "settings", request_body = ApprovalsSettings, responses((status = 200, body = ApprovalsSettings)))]
pub async fn update_approvals(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Json(body): Json<ApprovalsSettings>,
) -> Result<Json<ApprovalsSettings>, ApiError> {
    auth.require(Permission::SettingsWrite)?;
    let value =
        serde_json::to_value(&body).map_err(|error| ApiError::bad_request(error.to_string()))?;
    state
        .store
        .set_platform_setting(APPROVALS_SETTING_KEY, &value, Some(auth.user_id))
        .await?;
    crate::audit::record(
        &state,
        &auth,
        "settings.approvals.updated",
        Some("settings"),
        None,
        serde_json::json!({
            "approvalsRequireDifferentApprover": body.approvals_require_different_approver,
        }),
    )
    .await;
    Ok(Json(body))
}
