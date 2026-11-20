use axum::extract::State;
use axum::Extension;
use axum::Json;

use orbit_security::{AuthContext, Permission};

use crate::dto::DashboardResponse;
use crate::error::ApiError;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/v1/dashboard/summary", tag = "dashboard", responses((status = 200, body = DashboardResponse)))]
pub async fn dashboard_summary(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Json<DashboardResponse>, ApiError> {
    auth.require(Permission::AgentsRead)?;
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    let scope = access.store_access();
    let summary = state
        .store
        .dashboard_summary(auth.org_id, Some(&scope))
        .await?;
    let usage = state
        .store
        .usage_rollup(auth.org_id, 14, Some(&scope))
        .await?;
    Ok(Json(DashboardResponse { summary, usage }))
}
