# IDOR / access matrix

Enforced by `crates/orbit-api/src/access.rs` (`AccessScope`). Invisible resources return
`404`; insufficient write permission returns `403`.

| Resource | List filter | Get | Write | Execution/approval |
|---|---|---|---|---|
| Agents | yes | yes | yes | parent visibility |
| Skills | yes | yes | yes | — |
| Connections | yes | yes | yes | referenced connection checked on run |
| Providers | yes | yes | yes | — |
| Worker profiles | yes | yes | yes | claims scoped by owning team |
| Triggers | yes | yes | yes | — |
| Tool policies | yes | yes | yes | — |
| Executions | via agent | via agent | cancel via agent | events/SSE via agent |
| Approvals | via execution → agent | — | decide needs `approvals:decide` and use/manage on the agent (team viewers 403, invisible 404); optional four-eyes; expired → 409 | worker wait bound to its own execution |
| Env mappings | — | — | `settings:write`; each `secretRef`/`secretKey`/`connectionId` must be a connection/provider secret in the caller's org that the caller can use (else 422); resolution is org-scoped | — |
| Access teams | member list | member | lead/owner/admin | — |
| Sharing | — | view | manage + license | — |

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
