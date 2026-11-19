# RBAC

## Built-in roles

| Role | Can do |
|---|---|
| `owner` | everything, including `settings:write` |
| `admin` | everything except platform settings |
| `operator` | create/cancel executions, write agents/connections/triggers |
| `viewer` | read-only |

Permissions are checked in handlers with `auth.require(Permission::…)`. The platform
evaluates them from the stored role; custom roles carry an explicit permission array.

## Custom roles (enterprise)

`GET/POST /api/v1/roles`, `PATCH/DELETE /api/v1/roles/{id}`; mutations require
`settings:write` and the `custom_roles` license feature. Built-in role names are reserved;
permissions are validated against the catalog and audited. Assign with
`PATCH /api/v1/users/{id} {"role":"Release manager"}`.

The role name is stored on the user; at authentication time ORBIT resolves built-in
permissions or the custom permission set. Deleting a role locks assigned users out until
they are reassigned (403), rather than silently downgrading them.

## API tokens

API tokens never exceed their owner's current role: effective permissions are the token's
scopes intersected with the owner's present permissions, so demoting a user immediately
narrows their existing tokens. Minting a token with a scope you do not hold is rejected
(422); `*` means "whatever my role grants". See [docs/development/rbac.md](../development/rbac.md).

## OIDC group mapping

`roleClaim` + `groupRoles` map IdP groups to built-in roles; `syncRoles` re-evaluates on
login. See [docs/development/oidc.md](../development/oidc.md).

## Access teams and resource sharing (enterprise)

Access teams (`/api/v1/teams`, sidebar **Teams**) are available with the `teams` license
feature. Members have `lead`, `member` or `viewer` roles; the creator becomes lead. The
**Default** team cannot be deleted. These are distinct from **Agent Teams** (workflows).

Shareable resources — agents, skills, connections, providers, worker profiles, triggers
and tool policies — carry `ownerUserId`, `ownerTeamId` and `visibility`:

| Visibility | Visible to |
|---|---|
| `private` | owner and org owner/admin |
| `team` | members of the owning access team, plus owner/admin |
| `org` | every member of the organization |

`GET/PATCH /api/v1/sharing/{resource}/{id}` reads and changes sharing; mutations require
the `resource_permissions` feature and manage access, and are audited. Worker profiles
with an owning team only claim executions whose agent is `org`-visible or owned by that
team.

## Not implemented yet

Per-resource grants to individual users, shared connections with scoped access, and
marketplace entitlements. See #15.
