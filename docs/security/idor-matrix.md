# IDOR / access matrix

Enforced by `crates/orbit-api/src/access.rs` (`AccessScope`). Invisible resources return
`404`; insufficient write permission returns `403`.

| Resource | List filter | Get | Write | Execution/approval |
|---|---|---|---|---|
| Agents | yes | yes | yes | parent visibility |
| Agent versions (`/agents/{id}/versions`) | — | parent visibility (404) | — | — |
| Agent export / agent executions list | — | parent visibility (404) | — | — |
| Agent runtime / activate / pause | — | manage (404 if invisible) | manage | — |
| Skills | yes | yes | yes | — |
| Connections | yes | yes | yes | **use** required on every run path (see preflight) |
| Providers | yes | yes | yes | **use** required when an agent binds it, in both platform- and harness-managed models (incl. team subagents); the worker context re-checks the run principal before returning the credential |
| Worker profiles | yes | yes | yes | claims scoped by owning team |
| Triggers | yes | yes | yes | create needs **use** on the target agent and its spec references; fire re-checks the owner |
| Tool policies | yes | yes | yes | — |
| Executions | via agent | via agent | cancel via agent | events/SSE via agent |
| Approvals | via execution → agent | — | decision inherits execution | — |
| Dashboard summary | agent/execution/approval/connection/trigger counts, recent executions and usage filtered in SQL (`ResourceAccess`, joined via agent) | — | — | pending approvals via execution → agent |
| Access teams | member list | member | lead/owner/admin | — |
| Agent Team queue | team **view** | team **view** | team **use** (enqueue/run/retry/requeue/cancel) | — |
| Sharing | — | view | manage + license | — |
| Export | per-kind read permission + view | — | — | — |

Worker-surface rows (#139): a worker token acts only on executions it claimed. The claim
stores the token id; legacy rows claimed before binding (NULL token, worker id set) stay
reachable by org/profile scope until they finish. Tokens shared by several workers (e.g.
one profile token for a pool, or Docker-driver child containers) are one principal.

Owner/admin bypass is intentional and documented in ADR 0011.


## Approvals and secret references (#144)

- `POST /approvals/{id}/approve|reject` requires `approvals:decide` **and** use access to
  the agent (`AccessScope::require_use`). Owners/admins, the agent owner and team
  leads/members pass; team viewers receive `403`, users who cannot see the agent `404`.
- Four-eyes: `PUT /api/v1/settings/approvals {"approvalsRequireDifferentApprover": true}`
  (default `false`, API only) makes the user in `executions.requested_by` ineligible to
  approve or reject that execution's approvals (`403`).
- An approval past `expires_at` cannot be decided: the call returns `409 approval expired`
  and the row is marked `expired`.
- Worker `GET /worker/approvals/{id}/wait` returns `404` unless the approval belongs to the
  `executionId` in the query. `POST /worker/approvals` clamps `waitSeconds` to 1 s..24 h and
  allows at most 50 pending approvals per execution (`429`).
- Env-mapping secret refs are validated on create/update (`422`, same message for unknown
  and foreign refs, never including values) and resolved with
  `Store::get_secret_value_for_org`. The unscoped `get_secret_value` remains only for
  references read from org-checked rows and the org-less `SecretStore` trait.
