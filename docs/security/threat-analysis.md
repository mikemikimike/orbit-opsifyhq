# Threat Analysis — ORBIT

## Scope

This document covers the threat model for ORBIT as a self-hosted control plane
for AI agents. It identifies assets, threat actors, attack vectors, and mitigations.

## Assets

| Asset | Location | Sensitivity |
|---|---|---|
| Provider credentials (API keys) | PostgreSQL (encrypted at rest) | Critical |
| Connection credentials (GitHub/GitLab tokens) | PostgreSQL (encrypted at rest) | Critical |
| Worker tokens | PostgreSQL (hashed) | High |
| Agent definitions | PostgreSQL | Medium |
| Execution history (input/output) | PostgreSQL | Medium |
| Audit logs | PostgreSQL | Medium |
| Master key (ORBIT_MASTER_KEY) | Environment variable | Critical |
| Worker tokens (plaintext) | Worker process env | High |

## Threat actors

1. **External attacker** — no credentials, network access only
2. **Malicious agent** — compromised agent definition or prompt injection
3. **Insider threat** — authenticated user with excessive permissions
4. **Supply chain** — compromised dependency or container image

## Attack vectors and mitigations

### 1. Credential theft

**Vector:** Attacker extracts provider/connection credentials from the database.

**Mitigations:**
- Credentials are encrypted at rest (AES-256-GCM via `SecretCipher`)
- Master key is required to decrypt; stored as env var, not in DB
- API never returns plaintext credentials (`SecretLease` refuses serialization)
- Worker tokens are hashed (only the hash is stored)

### 2. Prompt injection

**Vector:** Malicious content in issues/emails causes the agent to execute unintended actions.

**Mitigations:**
- Autonomy is platform-enforced (`evaluate_capability`), not prompt-based
- Capability allowlists are enforced by the tool broker, not the harness
- External content is marked as untrusted in the execution input
- Write actions require approval (autonomy L3) or are auto-approved only at L4

### 3. Agent privilege escalation

**Vector:** Agent attempts to use capabilities it wasn't granted.

**Mitigations:**
- Tool broker enforces capability allowlists per execution
- `evaluate_capability()` checks autonomy level, approval mode, and grants
- Agents cannot modify their own definitions (requires SettingsWrite permission)
- Worker tokens are scoped to specific agents and harnesses

### 4. Data exfiltration via agent

**Vector:** Agent reads sensitive data and sends it to an external service.

**Mitigations:**
- Connectors are allowlisted per agent binding
- External side effects require approval at L3
- Audit logs record all tool calls with arguments
- Execution output is stored but not automatically shared

### 5. Container escape (worker)

**Vector:** Compromised agent escapes the worker container.

**Mitigations:**
- Workers run as non-root (uid 10001)
- Per-execution containers (K8s Jobs mode) provide isolation
- Read-only root filesystem where possible
- CPU/memory limits enforced
- Network egress: not enforced by ORBIT (agent `spec.resources.network` is recorded only); restrict with Kubernetes NetworkPolicy
- No database access from workers (token-only API)

### 6. Supply chain compromise

**Vector:** Compromised dependency or container image.

**Mitigations:**
- Container images scanned with Trivy in CI
- Rust dependencies audited with `cargo audit`
- Pinned versions in Cargo.lock and pnpm-lock.yaml
- Multi-stage builds minimize attack surface
- Images published to private registry (not public Docker Hub)

### 7. Unauthorized access to control plane

**Vector:** Attacker accesses the API without authentication.

**Mitigations:**
- All API routes require authentication (dev token or OIDC)
- Dev auth mode is disabled in production (`ORBIT_AUTH_MODE=dev` rejected when `ORBIT_ENV=production`)
- OIDC mode validates JWTs against provider's JWKS
- API tokens are hashed and scoped

### 8. Worker impersonation

**Vector:** Attacker uses a stolen worker token to impersonate a worker.

**Mitigations:**
- Worker tokens are scoped to specific agents and harnesses
- Tokens can be revoked
- Worker heartbeats are monitored (stale workers are reaped)
- Single-execution tokens for container mode

## Recommendations for production

1. Use OIDC authentication (not dev mode)
2. Enable network policies to restrict worker egress
3. Use per-execution containers (K8s Jobs) for isolation
4. Enable audit log monitoring
5. Rotate worker tokens periodically
6. Use a managed database with encryption at rest
7. Enable TLS for all internal communication
8. Set `ORBIT_MASTER_KEY` to a strong, unique value
EOF
echo done

## Tool policy layer

- Rules are additive restrictions. A compromised or prompt-injected agent cannot grant
  itself a capability: platform autonomy/grants are evaluated first and rule `allow`
  cannot weaken a platform `require_approval`/`deny`.
- Argument matchers normalize paths and parse shell commands. Paths are lexically
  normalized before matching so `../` cannot bypass a `secrets/**` rule; command rules
  use regexes validated at write time.
- Native tools with no matching rule default to `require_approval`, not allow. Approval
  decisions are bound to the execution, expire with it, and default-deny on timeout.
- Rule management requires `settings:write` (owner) and is audited.

### 9. Cross-team object access (IDOR)

**Threat:** an authenticated member guesses an object ID and reads or mutates another
team's agent, connection, provider, worker profile, trigger, tool policy, execution or
approval.

**Mitigations:**

- Every shareable row carries `owner_user_id`, `owner_team_id` and `visibility`
  (`private | team | org`); legacy rows are `org` in the Default team.
- `AccessScope` is loaded from the caller's role and team memberships. List endpoints
  filter rows; get/update/delete/export use `require_view`/`require_manage`, returning
  `404` for invisible rows to avoid confirming existence.
- Executions, events, SSE streams, cancellation and approvals are checked against the
  parent agent's visibility, not just org membership.
- Worker claims are constrained to the worker profile's owning team; a team worker cannot
  claim another team's private agent.
- Sharing changes require manage access plus the `resource_permissions` license feature;
  team writes require `teams`. Both are audited.
- Integration tests create two operator users with API tokens and assert list/get/delete
  isolation.

**Residual risk:** org owners/admins intentionally see all resources. Team-run objects
follow their agent team and are not yet independently shareable; see issue #37.
