//! Resource visibility sharing (ADR 0011).

use uuid::Uuid;

use crate::{Store, StoreError, StoreResult};

const SHAREABLE: &[&str] = &[
    "agents",
    "skills",
    "connections",
    "providers",
    "worker_profiles",
    "triggers",
    "tool_policies",
    "agent_teams",
];

fn table(resource: &str) -> StoreResult<&'static str> {
    SHAREABLE
        .iter()
        .copied()
        .find(|name| *name == resource)
        .ok_or_else(|| StoreError::Invalid(format!("resource '{resource}' is not shareable")))
}

impl Store {
    pub async fn resource_ownership(
        &self,
        org_id: Uuid,
        resource: &str,
        id: Uuid,
    ) -> StoreResult<(Option<Uuid>, Option<Uuid>, String)> {
        let table = table(resource)?;
        let query = format!(
            "SELECT owner_user_id, owner_team_id, visibility FROM {table} WHERE org_id = $1 AND id = $2"
        );
        // Safe: `table` comes from the SHAREABLE allowlist; all values are bound.
        sqlx::query_as::<_, (Option<Uuid>, Option<Uuid>, String)>(sqlx::AssertSqlSafe(query))
            .bind(org_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::NotFound(resource.into()))
    }

    pub async fn set_resource_visibility(
        &self,
        org_id: Uuid,
        resource: &str,
        id: Uuid,
        visibility: &str,
        owner_team_id: Option<Uuid>,
    ) -> StoreResult<()> {
        if !matches!(visibility, "private" | "team" | "org") {
            return Err(StoreError::Invalid(
                "visibility must be private, team or org".into(),
            ));
        }
        let table = table(resource)?;
        let query = format!(
            "UPDATE {table} SET visibility = $3, owner_team_id = $4, updated_at = now() WHERE org_id = $1 AND id = $2"
        );
        // Safe: `table` comes from the SHAREABLE allowlist; all values are bound.
        let result = sqlx::query(sqlx::AssertSqlSafe(query))
            .bind(org_id)
            .bind(id)
            .bind(visibility)
            .bind(owner_team_id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound(resource.into()));
        }
        Ok(())
    }
}

impl Store {
    /// Set the owner of a newly created resource when the insert did not carry one.
    pub async fn assign_resource_owner(
        &self,
        org_id: Uuid,
        resource: &str,
        id: Uuid,
        user_id: Uuid,
    ) -> StoreResult<()> {
        let table = table(resource)?;
        let query = format!(
            "UPDATE {table} SET owner_user_id = COALESCE(owner_user_id, $3), updated_at = now() WHERE org_id = $1 AND id = $2"
        );
        // Safe: `table` comes from the SHAREABLE allowlist; all values are bound.
        sqlx::query(sqlx::AssertSqlSafe(query))
            .bind(org_id)
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
