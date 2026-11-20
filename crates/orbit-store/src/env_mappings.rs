//! Reusable environment mappings shared across worker profiles.

use uuid::Uuid;

use crate::access::{binds, ResourceAccess};
use crate::models::EnvMapping;
use crate::{Store, StoreError, StoreResult};

impl Store {
    pub async fn list_env_mappings(
        &self,
        org_id: Uuid,
        access: Option<&ResourceAccess>,
    ) -> StoreResult<Vec<EnvMapping>> {
        let (privileged, user_id, team_ids, isolation) = binds(access);
        Ok(sqlx::query_as::<_, EnvMapping>(
            r#"
            SELECT * FROM env_mappings m
            WHERE m.org_id = $1
              AND ($2::bool
                   OR m.owner_user_id = $3
                   OR CASE WHEN $5::bool AND m.owner_team_id IS NOT NULL
                           THEN m.owner_team_id = ANY($4)
                           ELSE m.visibility = 'org'
                                OR (m.visibility = 'team' AND m.owner_team_id = ANY($4)) END)
            ORDER BY m.name ASC
            "#,
        )
        .bind(org_id)
        .bind(privileged)
        .bind(user_id)
        .bind(&team_ids)
        .bind(isolation)
        .fetch_all(&self.pool)
        .await?)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_env_mapping(
        &self,
        org_id: Uuid,
        name: &str,
        description: &str,
        mappings: &serde_json::Value,
    ) -> StoreResult<EnvMapping> {
        sqlx::query_as::<_, EnvMapping>(
            r#"
            INSERT INTO env_mappings (id, org_id, name, description, mappings)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(Uuid::now_v7())
        .bind(org_id)
        .bind(name)
        .bind(description)
        .bind(mappings)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| {
            if error
                .as_database_error()
                .map(|db| db.is_unique_violation())
                .unwrap_or(false)
            {
                StoreError::Conflict(format!("env mapping '{name}' already exists"))
            } else {
                error.into()
            }
        })
    }

    pub async fn update_env_mapping(
        &self,
        org_id: Uuid,
        id: Uuid,
        name: &str,
        description: &str,
        mappings: &serde_json::Value,
    ) -> StoreResult<EnvMapping> {
        sqlx::query_as::<_, EnvMapping>(
            r#"
            UPDATE env_mappings
            SET name = $3, description = $4, mappings = $5, updated_at = now()
            WHERE org_id = $1 AND id = $2
            RETURNING *
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(mappings)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| StoreError::NotFound("env mapping".into()))
    }

    pub async fn get_env_mapping(&self, org_id: Uuid, id: Uuid) -> StoreResult<EnvMapping> {
        sqlx::query_as::<_, EnvMapping>("SELECT * FROM env_mappings WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| StoreError::NotFound("env mapping".into()))
    }

    pub async fn delete_env_mapping(&self, org_id: Uuid, id: Uuid) -> StoreResult<()> {
        let result = sqlx::query("DELETE FROM env_mappings WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound("env mapping".into()));
        }
        Ok(())
    }

    pub async fn set_worker_profile_env_mappings(
        &self,
        org_id: Uuid,
        profile_id: Uuid,
        mapping_ids: &[Uuid],
    ) -> StoreResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM worker_profile_env_mappings WHERE profile_id = $1")
            .bind(profile_id)
            .execute(&mut *tx)
            .await?;
        for mapping_id in mapping_ids {
            let exists: Option<(Uuid,)> =
                sqlx::query_as("SELECT id FROM env_mappings WHERE org_id = $1 AND id = $2")
                    .bind(org_id)
                    .bind(mapping_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if exists.is_none() {
                return Err(StoreError::NotFound(format!(
                    "env mapping {mapping_id} is not in this organization"
                )));
            }
            sqlx::query(
                "INSERT INTO worker_profile_env_mappings (profile_id, env_mapping_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(profile_id)
            .bind(mapping_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_worker_profile_env_mappings(
        &self,
        profile_id: Uuid,
    ) -> StoreResult<Vec<EnvMapping>> {
        Ok(sqlx::query_as::<_, EnvMapping>(
            r#"
            SELECT e.* FROM env_mappings e
            JOIN worker_profile_env_mappings j ON j.env_mapping_id = e.id
            WHERE j.profile_id = $1
            ORDER BY e.name ASC
            "#,
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?)
    }
}
