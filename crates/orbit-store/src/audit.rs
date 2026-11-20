use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{AuditLogEntry, AuditLogRecord};
use crate::{Store, StoreResult};

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub async fn append_audit_log(
        &self,
        org_id: Uuid,
        actor_type: &str,
        actor_id: Option<Uuid>,
        actor_email: Option<&str>,
        action: &str,
        target_type: Option<&str>,
        target_id: Option<Uuid>,
        metadata: serde_json::Value,
        request_id: Option<&str>,
        ip_address: Option<&str>,
    ) -> StoreResult<()> {
        sqlx::query(
            r#"
            INSERT INTO audit_logs (org_id, actor_type, actor_id, actor_email, action,
                                    target_type, target_id, metadata, request_id, ip_address)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(org_id)
        .bind(actor_type)
        .bind(actor_id)
        .bind(actor_email)
        .bind(action)
        .bind(target_type)
        .bind(target_id)
        .bind(metadata)
        .bind(request_id)
        .bind(ip_address)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_audit(&self, org_id: Uuid, entry: AuditLogEntry) -> StoreResult<()> {
        self.append_audit_log(
            org_id,
            &entry.actor_type,
            entry.actor_id,
            entry.actor_email.as_deref(),
            &entry.action,
            entry.target_type.as_deref(),
            entry.target_id,
            entry.metadata,
            entry.request_id.as_deref(),
            entry.ip_address.as_deref(),
        )
        .await
    }

    /// Audit logs use a bigserial id cursor (append-only, monotonic).
    pub async fn list_audit_logs(
        &self,
        org_id: Uuid,
        limit: i64,
        before_id: Option<i64>,
        action: Option<&str>,
    ) -> StoreResult<(Vec<AuditLogRecord>, Option<i64>)> {
        let mut rows = sqlx::query_as::<_, AuditLogRecord>(
            r#"
            SELECT * FROM audit_logs
            WHERE org_id = $1
              AND ($2::text IS NULL OR action = $2)
              AND ($3::bigint IS NULL OR id < $3)
            ORDER BY id DESC
            LIMIT $4
            "#,
        )
        .bind(org_id)
        .bind(action)
        .bind(before_id)
        .bind(limit + 1)
        .fetch_all(&self.pool)
        .await?;
        let has_more = rows.len() as i64 > limit;
        if has_more {
            rows.truncate(limit as usize);
        }
        let next = if has_more {
            rows.last().map(|r| r.id)
        } else {
            None
        };
        Ok((rows, next))
    }

    /// Delete audit log rows created before `before`; returns the number removed.
    /// Driven by `ORBIT_AUDIT_RETENTION_DAYS` in the API background loop.
    pub async fn prune_audit_logs(&self, before: DateTime<Utc>) -> StoreResult<u64> {
        let result = sqlx::query("DELETE FROM audit_logs WHERE created_at < $1")
            .bind(before)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
