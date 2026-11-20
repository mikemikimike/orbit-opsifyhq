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


## API tokens

Tokens are always bounded by their owner's **current** role:

- Effective permissions = token scopes ∩ the owner's current permissions (the `*`
  wildcard is honoured on either side). Demoting a user, or editing their custom role,
  takes effect on their existing tokens immediately; no revocation is needed.
- A token whose scopes no longer overlap with the owner's role is rejected with `403`.
- An empty stored scope list means "the owner's role permissions".
- `POST /api/v1/api-tokens` rejects (`422`, offending scopes in `error.details`) any explicit
  scope the caller does not currently hold (`*` is allowed and resolves to the caller's
  current permissions); omitting `scopes` defaults to the caller's own
  permissions. Disabled users' tokens are rejected.

## Not implemented yet (issue #15)

- Resource-level permissions (per agent/connection/team)
- Shared connections with scoped access
- Quotas per org/user
- License-key gating for enterprise features
- Settings UI for role management (API only for now)
