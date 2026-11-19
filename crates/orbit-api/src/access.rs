//! Resource access scope for multi-team visibility (ADR 0011).

use std::collections::BTreeMap;

use uuid::Uuid;

use orbit_domain::UserRole;
use orbit_security::AuthContext;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct AccessScope {
    pub user_id: Uuid,
    pub role: UserRole,
    /// team_id -> role (`lead`, `member`, `viewer`)
    pub teams: BTreeMap<Uuid, String>,
    /// Non-admins only see resources owned by their teams.
    pub team_isolation: bool,
}

impl AccessScope {
    pub async fn load(state: &AppState, auth: &AuthContext) -> Result<Self, ApiError> {
        Self::build(state, auth.org_id, auth.user_id, auth.role).await
    }

    /// Access scope for a stored user identity, for code paths that run without an HTTP
    /// caller (scheduler, runtime ticks, team step advancement). Disabled users get
    /// `403`. Custom-role users are treated like viewers for resource scoping, matching
    /// how request authentication resolves them.
    pub async fn load_for_user(
        state: &AppState,
        org_id: Uuid,
        user_id: Uuid,
    ) -> Result<Self, ApiError> {
        let user = state.store.get_user(org_id, user_id).await?;
        if user.status != "active" {
            return Err(ApiError::forbidden("user is disabled"));
        }
        let role = user.role.parse::<UserRole>().unwrap_or(UserRole::Viewer);
        Self::build(state, org_id, user_id, role).await
    }

    async fn build(
        state: &AppState,
        org_id: Uuid,
        user_id: Uuid,
        role: UserRole,
    ) -> Result<Self, ApiError> {
        let teams = state
            .store
            .list_user_team_roles(org_id, user_id)
            .await?
            .into_iter()
            .collect();
        let team_isolation = state
            .store
            .get_platform_setting("team_isolation")
            .await
            .ok()
            .flatten()
            .and_then(|value| value.get("enabled").and_then(|enabled| enabled.as_bool()))
            .unwrap_or(true);
        Ok(Self {
            user_id,
            role,
            teams,
            team_isolation,
        })
    }

    fn is_privileged(&self) -> bool {
        matches!(self.role, UserRole::Owner | UserRole::Admin)
    }

    pub fn store_access(&self) -> orbit_store::access::ResourceAccess {
        // Lists must hide exactly what `can_view` hides, including team isolation.
        orbit_store::access::ResourceAccess::new(
            self.user_id,
            self.is_privileged(),
            self.teams.keys().copied().collect(),
        )
        .with_team_isolation(self.team_isolation)
    }

    fn leads_team(&self, team_id: Option<Uuid>) -> bool {
        team_id
            .and_then(|id| self.teams.get(&id))
            .is_some_and(|role| role == "lead")
    }

    pub fn can_view(
        &self,
        owner_user_id: Option<Uuid>,
        owner_team_id: Option<Uuid>,
        visibility: &str,
    ) -> bool {
        if self.is_privileged() {
            return true;
        }
        if owner_user_id == Some(self.user_id) {
            return true;
        }
        if self.team_isolation {
            if let Some(team_id) = owner_team_id {
                return self.teams.contains_key(&team_id);
            }
        }
        if self.leads_team(owner_team_id) {
            return true;
        }
        match visibility {
            "private" => false,
            "team" => owner_team_id.is_some_and(|id| self.teams.contains_key(&id)),
            _ => true,
        }
    }

    /// Use/run access: team leads and members can use, viewers cannot.
    pub fn can_use(
        &self,
        owner_user_id: Option<Uuid>,
        owner_team_id: Option<Uuid>,
        visibility: &str,
    ) -> bool {
        if self.is_privileged() || owner_user_id == Some(self.user_id) {
            return true;
        }
        if let Some(team_id) = owner_team_id {
            return matches!(
                self.teams.get(&team_id).map(String::as_str),
                Some("lead" | "member")
            );
        }
        self.can_view(owner_user_id, owner_team_id, visibility)
    }

    pub fn require_use(
        &self,
        resource: &str,
        owner_user_id: Option<Uuid>,
        owner_team_id: Option<Uuid>,
        visibility: &str,
    ) -> Result<(), ApiError> {
        if !self.can_view(owner_user_id, owner_team_id, visibility) {
            return Err(ApiError::not_found(resource));
        }
        if self.can_use(owner_user_id, owner_team_id, visibility) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "your team role cannot use this {resource}"
            )))
        }
    }

    pub fn can_manage(&self, owner_user_id: Option<Uuid>, owner_team_id: Option<Uuid>) -> bool {
        if self.is_privileged() || owner_user_id == Some(self.user_id) {
            return true;
        }
        self.leads_team(owner_team_id)
    }

    pub fn require_view(
        &self,
        resource: &str,
        owner_user_id: Option<Uuid>,
        owner_team_id: Option<Uuid>,
        visibility: &str,
    ) -> Result<(), ApiError> {
        if self.can_view(owner_user_id, owner_team_id, visibility) {
            Ok(())
        } else {
            // Do not reveal existence.
            Err(ApiError::not_found(resource))
        }
    }

    pub fn require_manage(
        &self,
        resource: &str,
        owner_user_id: Option<Uuid>,
        owner_team_id: Option<Uuid>,
        visibility: &str,
    ) -> Result<(), ApiError> {
        if !self.can_view(owner_user_id, owner_team_id, visibility) {
            return Err(ApiError::not_found(resource));
        }
        if self.can_manage(owner_user_id, owner_team_id) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "you cannot manage this {resource}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(role: UserRole, teams: &[(Uuid, &str)]) -> AccessScope {
        AccessScope {
            user_id: Uuid::now_v7(),
            role,
            teams: teams
                .iter()
                .map(|(id, role)| (*id, (*role).to_string()))
                .collect(),
            team_isolation: false,
        }
    }

    #[test]
    fn org_visibility_is_visible_to_members() {
        let access = scope(UserRole::Viewer, &[]);
        assert!(access.can_view(None, None, "org"));
    }

    #[test]
    fn private_resources_are_owner_only() {
        let access = scope(UserRole::Viewer, &[]);
        let other = Uuid::now_v7();
        assert!(access.can_view(Some(access.user_id), None, "private"));
        assert!(!access.can_view(Some(other), None, "private"));
        assert!(!access.can_manage(Some(other), None));
    }

    #[test]
    fn teams_see_their_resources_and_leads_manage() {
        let team = Uuid::now_v7();
        let member = scope(UserRole::Operator, &[(team, "member")]);
        assert!(member.can_view(None, Some(team), "team"));
        assert!(!member.can_manage(None, Some(team)));

        let lead = scope(UserRole::Operator, &[(team, "lead")]);
        assert!(lead.can_manage(None, Some(team)));
    }

    #[test]
    fn admins_and_owners_govern_everything() {
        for role in [UserRole::Owner, UserRole::Admin] {
            let access = scope(role, &[]);
            assert!(access.can_view(Some(Uuid::now_v7()), Some(Uuid::now_v7()), "private"));
            assert!(access.can_manage(Some(Uuid::now_v7()), Some(Uuid::now_v7())));
        }
    }

    #[test]
    fn team_isolation_hides_other_teams_even_for_org_visibility() {
        let team_a = Uuid::now_v7();
        let team_b = Uuid::now_v7();
        let mut access = scope(UserRole::Operator, &[(team_a, "member")]);
        access.team_isolation = true;
        assert!(access.can_view(None, Some(team_a), "org"));
        assert!(!access.can_view(None, Some(team_b), "org"));
        assert!(!access.can_view(None, None, "private"));
    }

    #[test]
    fn team_viewers_can_view_but_not_use() {
        let team = Uuid::now_v7();
        let access = scope(UserRole::Operator, &[(team, "viewer")]);
        assert!(access.can_view(None, Some(team), "org"));
        assert!(!access.can_use(None, Some(team), "org"));
        let member = scope(UserRole::Operator, &[(team, "member")]);
        assert!(member.can_use(None, Some(team), "org"));
    }
}
