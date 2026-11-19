//! Agent memory persistence.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{Store, StoreResult};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AgentMemoryEntry {
    pub id: Uuid,
    pub org_id: Uuid,
    pub agent_id: Uuid,
    pub kind: String,
    pub content: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Store {
    pub async fn store_memory(
        &self,
        org_id: Uuid,
        agent_id: Uuid,
        kind: &str,
        content: &str,
        metadata: &serde_json::Value,
        expires_at: Option<DateTime<Utc>>,
    ) -> StoreResult<AgentMemoryEntry> {
        sqlx::query_as::<_, AgentMemoryEntry>(
            r#"
            INSERT INTO agent_memory (id, org_id, agent_id, kind, content, metadata, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(orbit_domain::new_id())
        .bind(org_id)
        .bind(agent_id)
        .bind(kind)
        .bind(content)
        .bind(metadata)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(Into::into)
    }

    pub async fn list_memory(
        &self,
        org_id: Uuid,
        agent_id: Uuid,
        kind: Option<&str>,
        limit: i64,
    ) -> StoreResult<Vec<AgentMemoryEntry>> {
        Ok(sqlx::query_as::<_, AgentMemoryEntry>(
            r#"
            SELECT * FROM agent_memory
            WHERE org_id = $1 AND agent_id = $2
              AND ($3::text IS NULL OR kind = $3)
              AND (expires_at IS NULL OR expires_at > now())
            ORDER BY created_at DESC
            LIMIT $4
            "#,
        )
        .bind(org_id)
        .bind(agent_id)
        .bind(kind)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn prune_expired_memory(&self) -> StoreResult<u64> {
        let result = sqlx::query(
            "DELETE FROM agent_memory WHERE expires_at IS NOT NULL AND expires_at < now()",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}
