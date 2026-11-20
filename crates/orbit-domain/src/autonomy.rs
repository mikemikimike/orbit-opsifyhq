use serde::{Deserialize, Serialize};

/// Platform-enforced autonomy levels. The model never decides these; ORBIT does.
///
/// - 1 Observe: read-only capabilities; writes and side effects are denied.
/// - 2 Recommend: analyse and propose; writes and side effects are denied (no approval path).
/// - 3 Prepare: writes/side effects require explicit approval.
/// - 4 Autonomous: allowlisted writes/side effects execute without approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AutonomyLevel(u8);

impl AutonomyLevel {
    pub const OBSERVE: Self = Self(1);
    pub const RECOMMEND: Self = Self(2);
    pub const PREPARE: Self = Self(3);
    pub const AUTONOMOUS: Self = Self(4);

    pub fn from_level(level: u8) -> Option<Self> {
        (1..=4).contains(&level).then_some(Self(level))
    }

    pub fn level(&self) -> u8 {
        self.0
    }

    pub fn label(&self) -> &'static str {
        match self.0 {
            1 => "Observe",
            2 => "Recommend",
            3 => "Prepare",
            _ => "Autonomous",
        }
    }

    pub fn description(&self) -> &'static str {
        match self.0 {
            1 => "Read-only access to connected services; writes are denied.",
            2 => "Analyse and propose actions; writes are denied (raise to Prepare for approval-gated writes).",
            3 => "Prepare writes and side effects; a human must approve.",
            _ => "Execute allowlisted actions within platform policies.",
        }
    }

    /// Could this level ever execute the given capability effect without approval?
    pub fn permits_effect_without_approval(&self, effect: CapabilityEffect) -> bool {
        match effect {
            CapabilityEffect::Read => true,
            CapabilityEffect::Write | CapabilityEffect::ExternalSideEffect => self.0 >= 4,
        }
    }
}

impl Default for AutonomyLevel {
    fn default() -> Self {
        Self::RECOMMEND
    }
}

impl std::fmt::Display for AutonomyLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.0, self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEffect {
    Read,
    Write,
    ExternalSideEffect,
}

impl CapabilityEffect {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::ExternalSideEffect => "external_side_effect",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "external_side_effect" => Some(Self::ExternalSideEffect),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    RequireApproval,
    Deny,
}

impl PolicyDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireApproval => "require_approval",
            Self::Deny => "deny",
        }
    }
}

/// A capability granted to an agent by its platform configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantedCapability {
    pub connection_id: uuid::Uuid,
    pub connector: String,
    pub capability: String,
    pub effect: CapabilityEffect,
}

/// Evaluate a capability invocation against autonomy, approval mode and allowlist.
///
/// `granted` is derived from the agent definition's connector bindings and the
/// connection's capability catalog. An empty allowlist never grants writes.
///
/// Levels 1 (Observe) and 2 (Recommend) **deny** every write or external side
/// effect outright; there is no approval path, regardless of `approval_mode`.
/// Approval-gated writes start at level 3 (Prepare).
pub fn evaluate_capability(
    level: AutonomyLevel,
    approval_mode: crate::ApprovalMode,
    requested: &str,
    effect: CapabilityEffect,
    granted: &[GrantedCapability],
) -> PolicyDecision {
    let allowlisted = granted.iter().any(|g| g.capability == requested);
    if !allowlisted {
        return PolicyDecision::Deny;
    }
    match effect {
        CapabilityEffect::Read => PolicyDecision::Allow,
        CapabilityEffect::Write | CapabilityEffect::ExternalSideEffect
            if level < AutonomyLevel::PREPARE =>
        {
            PolicyDecision::Deny
        }
        CapabilityEffect::Write | CapabilityEffect::ExternalSideEffect => {
            let requires = match approval_mode {
                crate::ApprovalMode::Always => true,
                crate::ApprovalMode::Writes => {
                    effect == CapabilityEffect::ExternalSideEffect
                        || effect == CapabilityEffect::Write
                }
                crate::ApprovalMode::Auto => !level.permits_effect_without_approval(effect),
            };
            if requires {
                PolicyDecision::RequireApproval
            } else {
                PolicyDecision::Allow
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ApprovalMode;

    fn grant(capability: &str, effect: CapabilityEffect) -> GrantedCapability {
        GrantedCapability {
            connection_id: uuid::Uuid::now_v7(),
            connector: "gitlab".into(),
            capability: capability.into(),
            effect,
        }
    }

    #[test]
    fn observe_is_read_only() {
        let granted = vec![
            grant("issues.list", CapabilityEffect::Read),
            grant("issues.create", CapabilityEffect::Write),
        ];
        let level = AutonomyLevel::OBSERVE;
        assert_eq!(
            evaluate_capability(
                level,
                ApprovalMode::Auto,
                "issues.list",
                CapabilityEffect::Read,
                &granted
            ),
            PolicyDecision::Allow
        );
        assert_eq!(
            evaluate_capability(
                level,
                ApprovalMode::Auto,
                "issues.create",
                CapabilityEffect::Write,
                &granted
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn observe_and_recommend_deny_writes_in_every_approval_mode() {
        let granted = vec![
            grant("issues.create", CapabilityEffect::Write),
            grant("messages.send", CapabilityEffect::ExternalSideEffect),
            grant("issues.list", CapabilityEffect::Read),
        ];
        for level in [AutonomyLevel::OBSERVE, AutonomyLevel::RECOMMEND] {
            for mode in [
                ApprovalMode::Auto,
                ApprovalMode::Writes,
                ApprovalMode::Always,
            ] {
                for (cap, effect) in [
                    ("issues.create", CapabilityEffect::Write),
                    ("messages.send", CapabilityEffect::ExternalSideEffect),
                ] {
                    assert_eq!(
                        evaluate_capability(level, mode, cap, effect, &granted),
                        PolicyDecision::Deny,
                        "level {level} mode {mode:?} cap {cap}"
                    );
                }
                assert_eq!(
                    evaluate_capability(
                        level,
                        mode,
                        "issues.list",
                        CapabilityEffect::Read,
                        &granted
                    ),
                    PolicyDecision::Allow
                );
            }
        }
    }

    #[test]
    fn recommend_write_is_denied() {
        let granted = vec![grant("issues.create", CapabilityEffect::Write)];
        assert_eq!(
            evaluate_capability(
                AutonomyLevel::RECOMMEND,
                ApprovalMode::Auto,
                "issues.create",
                CapabilityEffect::Write,
                &granted
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn prepare_write_requires_approval() {
        let granted = vec![grant("issues.create", CapabilityEffect::Write)];
        for mode in [
            ApprovalMode::Auto,
            ApprovalMode::Writes,
            ApprovalMode::Always,
        ] {
            assert_eq!(
                evaluate_capability(
                    AutonomyLevel::PREPARE,
                    mode,
                    "issues.create",
                    CapabilityEffect::Write,
                    &granted
                ),
                PolicyDecision::RequireApproval
            );
        }
    }

    #[test]
    fn unlisted_capabilities_are_denied() {
        let granted = vec![grant("issues.list", CapabilityEffect::Read)];
        assert_eq!(
            evaluate_capability(
                AutonomyLevel::AUTONOMOUS,
                ApprovalMode::Auto,
                "issues.delete",
                CapabilityEffect::Write,
                &granted
            ),
            PolicyDecision::Deny
        );
    }

    #[test]
    fn autonomous_writes_execute() {
        let granted = vec![grant("messages.send", CapabilityEffect::ExternalSideEffect)];
        assert_eq!(
            evaluate_capability(
                AutonomyLevel::AUTONOMOUS,
                ApprovalMode::Auto,
                "messages.send",
                CapabilityEffect::ExternalSideEffect,
                &granted
            ),
            PolicyDecision::Allow
        );
        // ... unless the operator forces approvals.
        assert_eq!(
            evaluate_capability(
                AutonomyLevel::AUTONOMOUS,
                ApprovalMode::Always,
                "messages.send",
                CapabilityEffect::ExternalSideEffect,
                &granted
            ),
            PolicyDecision::RequireApproval
        );
    }

    #[test]
    fn levels_roundtrip() {
        for level in 1..=4 {
            assert_eq!(AutonomyLevel::from_level(level).unwrap().level(), level);
        }
        assert!(AutonomyLevel::from_level(0).is_none());
        assert!(AutonomyLevel::from_level(5).is_none());
    }
}
