use axum::extract::{Path, Query, State};
use axum::Extension;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use orbit_security::{AuthContext, Permission};
use orbit_store::Approval;

use crate::audit;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ApprovalListQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalDecisionRequest {
    #[serde(default)]
    pub comment: Option<String>,
}

#[utoipa::path(get, path = "/api/v1/approvals", tag = "approvals", params(ApprovalListQuery), responses((status = 200, body = [orbit_store::ApprovalWithExecution])))]
pub async fn list_approvals(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<ApprovalListQuery>,
) -> Result<Json<Vec<orbit_store::ApprovalWithExecution>>, ApiError> {
    auth.require(Permission::ApprovalsDecide)?;
    let status = query.status.as_deref().filter(|s| !s.is_empty());
    if let Some(status) = status {
        if !matches!(status, "pending" | "approved" | "rejected" | "expired") {
            return Err(ApiError::bad_request("unknown approval status"));
        }
    }
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    let store_access = access.store_access();
    let approvals = state
        .store
        .list_approvals(
            auth.org_id,
            status,
            query.limit.unwrap_or(50).clamp(1, 200) as i64,
            Some(&store_access),
        )
        .await?;
    Ok(Json(approvals))
}

#[utoipa::path(post, path = "/api/v1/approvals/{approval_id}/approve", tag = "approvals", request_body = ApprovalDecisionRequest, responses((status = 200, body = Approval)))]
pub async fn approve(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(approval_id): Path<Uuid>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<ApprovalDecisionRequest>,
) -> Result<Json<Approval>, ApiError> {
    decide(state, auth, approval_id, "approved", body.comment).await
}

#[utoipa::path(post, path = "/api/v1/approvals/{approval_id}/reject", tag = "approvals", request_body = ApprovalDecisionRequest, responses((status = 200, body = Approval)))]
pub async fn reject(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(approval_id): Path<Uuid>,
    crate::extract::ApiJson(body): crate::extract::ApiJson<ApprovalDecisionRequest>,
) -> Result<Json<Approval>, ApiError> {
    decide(state, auth, approval_id, "rejected", body.comment).await
}

async fn decide(
    state: AppState,
    auth: AuthContext,
    approval_id: Uuid,
    decision: &str,
    comment: Option<String>,
) -> Result<Json<Approval>, ApiError> {
    auth.require(Permission::ApprovalsDecide)?;
    let existing = state.store.get_approval(auth.org_id, approval_id).await?;
    let execution = state
        .store
        .get_execution(auth.org_id, existing.execution_id)
        .await?;
    let agent = state
        .store
        .get_agent(auth.org_id, execution.agent_id)
        .await?;
    let access = crate::access::AccessScope::load(&state, &auth).await?;
    access.require_view(
        "approval",
        agent.owner_user_id,
        agent.owner_team_id,
        &agent.visibility,
    )?;
    let approval = state
        .store
        .decide_approval(
            auth.org_id,
            approval_id,
            decision,
            auth.user_id,
            comment.as_deref(),
        )
        .await?;
    audit::record(
        &state,
        &auth,
        if decision == "approved" {
            "approval.approved"
        } else {
            "approval.rejected"
        },
        Some("approval"),
        Some(approval.id),
        serde_json::json!({
            "executionId": approval.execution_id,
            "comment": comment,
        }),
    )
    .await;
    Ok(Json(approval))
}
