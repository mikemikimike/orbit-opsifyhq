//! Natural-key lookups used by portable export/import. Documents reference other
//! resources by name/slug; these helpers translate those keys to ids.

use uuid::Uuid;

use crate::{Store, StoreError, StoreResult};

impl Store {
    /// Find a resource id by its portable natural key within an organisation.
    ///
    /// Keys: agents/providers/worker_profiles/agent_teams/env_mappings by `name`,
    /// skills by `slug`.
    pub async fn find_resource_id_by_key(
        &self,
        org_id: Uuid,
        resource: &str,
        key: &str,
    ) -> StoreResult<Option<Uuid>> {
        let (table, column) = match resource {
            "agents" => ("agents", "name"),
            "providers" => ("providers", "name"),
            "worker_profiles" => ("worker_profiles", "name"),
            "agent_teams" => ("agent_teams", "name"),
            "env_mappings" => ("env_mappings", "name"),
            "skills" => ("skills", "slug"),
            other => return Err(StoreError::Invalid(format!("no natural key for '{other}'"))),
        };
        let query = format!(
            "SELECT id FROM {table} WHERE org_id = $1 AND {column} = $2 ORDER BY created_at LIMIT 1"
        );
        Ok(sqlx::query_scalar::<_, Uuid>(&query)
            .bind(org_id)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Connections are unique per `(connector, name)`.
    pub async fn find_connection_id(
        &self,
        org_id: Uuid,
        connector_slug: &str,
        name: &str,
    ) -> StoreResult<Option<Uuid>> {
        Ok(sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT c.id FROM connections c
            JOIN connectors conn ON conn.id = c.connector_id
            WHERE c.org_id = $1 AND conn.slug = $2 AND c.name = $3
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .bind(connector_slug)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Tool policies are unique per `(agent or organisation, name)`.
    pub async fn find_tool_policy_id(
        &self,
        org_id: Uuid,
        agent_id: Option<Uuid>,
        name: &str,
    ) -> StoreResult<Option<Uuid>> {
        Ok(sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM tool_policies WHERE org_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND name = $3",
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?)
    }
}
