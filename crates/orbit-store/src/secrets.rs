use uuid::Uuid;

use orbit_security::{SecretStore, SecretValue};

use crate::{Store, StoreError, StoreResult};

/// Owning resource of a stored secret (see [`Store::secret_owner`]).
#[derive(Debug, Clone)]
pub struct SecretOwner {
    pub kind: &'static str,
    pub owner_user_id: Option<Uuid>,
    pub owner_team_id: Option<Uuid>,
    pub visibility: String,
}

impl Store {
    /// Store (or replace) an encrypted secret. Secrets are always encrypted at rest:
    /// a store without a configured cipher refuses writes.
    pub async fn put_secret(
        &self,
        org_id: Option<Uuid>,
        reference: &str,
        value: &SecretValue,
        description: &str,
    ) -> StoreResult<()> {
        let ciphertext = self.cipher()?.encrypt(reference, value)?;
        sqlx::query(
            r#"
            INSERT INTO secrets (reference, org_id, ciphertext, description)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (reference)
            DO UPDATE SET ciphertext = EXCLUDED.ciphertext,
                          org_id = EXCLUDED.org_id,
                          description = EXCLUDED.description,
                          updated_at = now()
            "#,
        )
        .bind(reference)
        .bind(org_id)
        .bind(ciphertext)
        .bind(description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Org-scoped read: only returns a secret stored for `org_id`. Use this wherever the
    /// reference came from user-editable data (env mappings, profiles) so a foreign
    /// reference can never be resolved across tenants.
    pub async fn get_secret_value_for_org(
        &self,
        org_id: Uuid,
        reference: &str,
    ) -> StoreResult<Option<SecretValue>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT ciphertext FROM secrets WHERE reference = $1 AND org_id = $2")
                .bind(reference)
                .bind(org_id)
                .fetch_optional(&self.pool)
                .await?;
        match row {
            None => Ok(None),
            Some((ciphertext,)) => {
                let value = self
                    .cipher()?
                    .decrypt(reference, &ciphertext)
                    .map_err(|e| StoreError::Internal(e.to_string()))?;
                Ok(Some(value))
            }
        }
    }

    /// Which resource owns `reference` in `org_id`, if the secret exists for that org.
    /// Outputs carry no access columns, so they report org-wide defaults.
    pub async fn secret_owner(
        &self,
        org_id: Uuid,
        reference: &str,
    ) -> StoreResult<Option<SecretOwner>> {
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT reference FROM secrets WHERE reference = $1 AND org_id = $2")
                .bind(reference)
                .bind(org_id)
                .fetch_optional(&self.pool)
                .await?;
        if exists.is_none() {
            return Ok(None);
        }
        type Row = (Option<Uuid>, Option<Uuid>, String);
        let connection: Option<Row> = sqlx::query_as(
            "SELECT owner_user_id, owner_team_id, visibility FROM connections WHERE org_id = $1 AND credential_ref = $2 LIMIT 1",
        )
        .bind(org_id)
        .bind(reference)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((owner_user_id, owner_team_id, visibility)) = connection {
            return Ok(Some(SecretOwner {
                kind: "connection",
                owner_user_id,
                owner_team_id,
                visibility,
            }));
        }
        let provider: Option<Row> = sqlx::query_as(
            "SELECT owner_user_id, owner_team_id, visibility FROM providers WHERE org_id = $1 AND secret_ref = $2 LIMIT 1",
        )
        .bind(org_id)
        .bind(reference)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((owner_user_id, owner_team_id, visibility)) = provider {
            return Ok(Some(SecretOwner {
                kind: "provider",
                owner_user_id,
                owner_team_id,
                visibility,
            }));
        }
        let output: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM outputs WHERE org_id = $1 AND secret_ref = $2 LIMIT 1")
                .bind(org_id)
                .bind(reference)
                .fetch_optional(&self.pool)
                .await?;
        Ok(output.map(|_| SecretOwner {
            kind: "output",
            owner_user_id: None,
            owner_team_id: None,
            visibility: "org".into(),
        }))
    }

    /// Unscoped read. Only for callers that already got the reference from an
    /// org-checked row (connection/provider/output fetched with `org_id`) and for the
    /// `SecretStore` trait, which has no org context. Never pass user-supplied references.
    pub async fn get_secret_value(&self, reference: &str) -> StoreResult<Option<SecretValue>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT ciphertext FROM secrets WHERE reference = $1")
                .bind(reference)
                .fetch_optional(&self.pool)
                .await?;
        match row {
            None => Ok(None),
            Some((ciphertext,)) => {
                let value = self
                    .cipher()?
                    .decrypt(reference, &ciphertext)
                    .map_err(|e| StoreError::Internal(e.to_string()))?;
                Ok(Some(value))
            }
        }
    }

    pub async fn delete_secret(&self, reference: &str) -> StoreResult<()> {
        sqlx::query("DELETE FROM secrets WHERE reference = $1")
            .bind(reference)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl SecretStore for Store {
    async fn put(
        &self,
        reference: &str,
        value: SecretValue,
    ) -> Result<(), orbit_security::SecurityError> {
        self.put_secret(None, reference, &value, "")
            .await
            .map_err(|e| orbit_security::SecurityError::Secret(e.to_string()))
    }

    async fn get(
        &self,
        reference: &str,
    ) -> Result<Option<SecretValue>, orbit_security::SecurityError> {
        self.get_secret_value(reference)
            .await
            .map_err(|e| orbit_security::SecurityError::Secret(e.to_string()))
    }

    async fn delete(&self, reference: &str) -> Result<(), orbit_security::SecurityError> {
        self.delete_secret(reference)
            .await
            .map_err(|e| orbit_security::SecurityError::Secret(e.to_string()))
    }
}
