# Roles and permissions

## Built-in roles

| Role | Permissions |
|---|---|
| `owner` | all permissions, including `settings:write` |
| `admin` | everything except `settings:write` |
| `operator` | read + agent/connection/trigger writes and execution create/cancel |
| `viewer` | read-only |

Built-in roles are virtual; permission checks use `orbit_security::Permission::allows`.

## Custom roles

Owners can define organization roles with an explicit permission set:

```bash
curl -X POST "$API/api/v1/roles" \
  -H "Authorization: Bearer $ORBIT_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
    "name": "Release manager",
    "permissions": ["agents:read", "executions:read", "executions:create", "approvals:decide"]
  }'
```

- `GET /api/v1/roles`, `POST /api/v1/roles`, `PATCH/DELETE /api/v1/roles/{id}`
- List requires `settings:read`; mutations require `settings:write` (owner)
- Names of built-in roles are reserved
- Permissions are validated against the platform permission catalog and audited
- Assign the role to a user by name: `PATCH /api/v1/users/{id} {"role":"Release manager"}`

At authentication time ORBIT resolves the stored role name:

- built-in → the role's permission set
- custom → the stored permission array (carried as the auth scopes; `auth.require`
  evaluates them like any other scope)
- deleted/unknown custom role → the request is rejected (`403`) until the user is
  reassigned; built-in roles are never affected

Custom roles currently apply to organization-level actions only. Roles are visible through
`GET /api/v1/me` via the effective `permissions` list.

## Resource access checks on top of roles

Roles gate organization-level actions; resource access (`AccessScope`, ADR 0011) gates the
individual agent, connection, provider or team. Running something needs **use** (team
`lead`/`member`, the owner, or admin), not just **view**. Notably:

- Every execution path (execute, rerun, runtime, scheduled triggers, team runs and queue
  starts) runs the shared preflight in `crates/orbit-api/src/preflight.rs`: agent use,
  use of every referenced connection and platform provider, harness availability and the
  hourly quota. Scheduler and runtime ticks re-check this for the stored owner at run time.
- Agent create/update/import require use of every connection and provider the spec binds.
- Cancelling an execution and creating a team run require use.
- Tool policies for an agent require manage on that agent; org-wide policies require
  `settings:write`.
- `GET /api/v1/export/{resource}/{id}` requires the read permission of the resource kind
  (`providers:read`, `connections:read`, `harnesses:read` for worker profiles and teams,
  `settings:read` for tool policies, `agents:read` otherwise) plus visibility.

See [the access matrix](../security/idor-matrix.md).

## Not implemented yet (issue #15)

- Resource-level permissions (per agent/connection/team)
- Shared connections with scoped access
- Quotas per org/user
- License-key gating for enterprise features
- Settings UI for role management (API only for now)
