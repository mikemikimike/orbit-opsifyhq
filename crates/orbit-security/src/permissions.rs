use orbit_domain::UserRole;
use serde::{Deserialize, Serialize};

/// Coarse-grained platform permissions checked in handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    AgentsRead,
    AgentsWrite,
    ExecutionsRead,
    ExecutionsCreate,
    ExecutionsCancel,
    ApprovalsDecide,
    HarnessesRead,
    ProvidersRead,
    ProvidersWrite,
    ConnectionsRead,
    ConnectionsWrite,
    TriggersWrite,
    SettingsRead,
    SettingsWrite,
    MembersRead,
    AuditRead,
}

impl Permission {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentsRead => "agents:read",
            Self::AgentsWrite => "agents:write",
            Self::ExecutionsRead => "executions:read",
            Self::ExecutionsCreate => "executions:create",
            Self::ExecutionsCancel => "executions:cancel",
            Self::ApprovalsDecide => "approvals:decide",
            Self::HarnessesRead => "harnesses:read",
            Self::ProvidersRead => "providers:read",
            Self::ProvidersWrite => "providers:write",
            Self::ConnectionsRead => "connections:read",
            Self::ConnectionsWrite => "connections:write",
            Self::TriggersWrite => "triggers:write",
            Self::SettingsRead => "settings:read",
            Self::SettingsWrite => "settings:write",
            Self::MembersRead => "members:read",
            Self::AuditRead => "audit:read",
        }
    }

    pub fn allows(role: UserRole, permission: Permission) -> bool {
        use Permission::*;
        match role {
            UserRole::Viewer => matches!(
                permission,
                AgentsRead
                    | ExecutionsRead
                    | HarnessesRead
                    | ProvidersRead
                    | ConnectionsRead
                    | SettingsRead
                    | MembersRead
            ),
            UserRole::Operator => matches!(
                permission,
                AgentsRead
                    | AgentsWrite
                    | ExecutionsRead
                    | ExecutionsCreate
                    | ExecutionsCancel
                    | HarnessesRead
                    | ProvidersRead
                    | ConnectionsRead
                    | ConnectionsWrite
                    | TriggersWrite
                    | SettingsRead
                    | MembersRead
            ),
            UserRole::Admin => !matches!(permission, SettingsWrite),
            UserRole::Owner => true,
        }
    }

    pub fn all_for(role: UserRole) -> Vec<&'static str> {
        const ALL: [Permission; 16] = [
            Permission::AgentsRead,
            Permission::AgentsWrite,
            Permission::ExecutionsRead,
            Permission::ExecutionsCreate,
            Permission::ExecutionsCancel,
            Permission::ApprovalsDecide,
            Permission::HarnessesRead,
            Permission::ProvidersRead,
            Permission::ProvidersWrite,
            Permission::ConnectionsRead,
            Permission::ConnectionsWrite,
            Permission::TriggersWrite,
            Permission::SettingsRead,
            Permission::SettingsWrite,
            Permission::MembersRead,
            Permission::AuditRead,
        ];
        ALL.into_iter()
            .filter(|p| Permission::allows(role, *p))
            .map(|p| p.as_str())
            .collect()
    }
}

/// Whether a permission set (which may contain the `*` wildcard) grants `scope`.
/// A requested `*` is only granted by a set that itself holds `*`.
pub fn scope_granted(held: &[String], scope: &str) -> bool {
    held.iter().any(|h| h == "*" || h == scope)
}

/// Intersect a token's scopes with the owner's current permissions, handling the `*`
/// wildcard on either side. The result never grants more than `held`.
pub fn intersect_scopes(token_scopes: &[String], held: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for scope in token_scopes {
        if scope == "*" {
            // Wildcard token: the owner's current set applies in full.
            for h in held {
                if !out.contains(h) {
                    out.push(h.clone());
                }
            }
        } else if scope_granted(held, scope) && !out.contains(scope) {
            out.push(scope.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewers_cannot_write() {
        assert!(Permission::allows(UserRole::Viewer, Permission::AgentsRead));
        assert!(!Permission::allows(
            UserRole::Viewer,
            Permission::AgentsWrite
        ));
        assert!(Permission::allows(
            UserRole::Operator,
            Permission::ExecutionsCreate
        ));
        assert!(!Permission::allows(
            UserRole::Operator,
            Permission::ProvidersWrite
        ));
        assert!(Permission::allows(
            UserRole::Admin,
            Permission::ProvidersWrite
        ));
        assert!(Permission::allows(
            UserRole::Owner,
            Permission::SettingsWrite
        ));
    }

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn intersection_drops_scopes_the_owner_lost() {
        let held = v(&["agents:read", "settings:read"]);
        let token = v(&["agents:read", "settings:write"]);
        assert_eq!(intersect_scopes(&token, &held), v(&["agents:read"]));
    }

    #[test]
    fn intersection_handles_wildcards_both_ways() {
        let held = v(&["agents:read"]);
        assert_eq!(intersect_scopes(&v(&["*"]), &held), held);
        assert_eq!(
            intersect_scopes(&v(&["agents:read", "audit:read"]), &v(&["*"])),
            v(&["agents:read", "audit:read"])
        );
        assert_eq!(intersect_scopes(&v(&["*"]), &v(&["*"])), v(&["*"]));
        assert!(intersect_scopes(&v(&["audit:read"]), &held).is_empty());
    }

    #[test]
    fn wildcard_scope_requires_wildcard_holder() {
        assert!(!scope_granted(&v(&["agents:read"]), "*"));
        assert!(scope_granted(&v(&["*"]), "*"));
        assert!(scope_granted(&v(&["agents:read"]), "agents:read"));
    }
}
