# IDOR / access matrix

Enforced by `crates/orbit-api/src/access.rs` (`AccessScope`). Invisible resources return
`404`; insufficient write permission returns `403`.

| Resource | List filter | Get | Write | Execution/approval |
|---|---|---|---|---|
| Agents | yes | yes | yes (author must be able to **use** every referenced connection and provider) | parent visibility; run needs **use** |
| Skills | yes | yes | yes | — |
| Connections | yes | yes | yes | **use** required on every run path (see preflight) |
| Providers | yes | yes | yes | **use** required when an agent binds it (platform-managed models) |
| Worker profiles | yes | yes | yes | claims scoped by owning team |
| Triggers | yes | yes | yes | re-checked at fire time for the owner |
| Tool policies | yes | yes | yes (agent-scoped rules need **manage** on the agent; org-wide rules need `settings:write`) | — |
| Executions | via agent | via agent | cancel needs **use** on the agent | events/SSE via agent |
| Approvals | via execution → agent | — | decision inherits execution | — |
| Access teams | member list | member | lead/owner/admin | — |
| Sharing | — | view | manage + license | — |
| Export | per-kind read permission + view | — | — | — |

Owner/admin bypass is intentional and documented in ADR 0011.

## Execution preflight

Every path that starts an execution goes through `crates/orbit-api/src/preflight.rs`
(`Preflight::authorize_access`, `check_harness`, `check_quota`, `run`; team runs also use
`authorize_team_run`). It checks, in order: agent not archived and **use** on the agent,
**use** on every connection bound in the spec (and a team's shared/work-source
connections) and on the platform model provider, harness enabled and `available`, and the
agent's `autonomy.maxExecutionsPerHour` quota (`429`).

| Path | Identity | Notes |
|---|---|---|
| `POST /agents/{id}/executions` | request caller | quota evaluated after the idempotent replay |
| `POST /executions/{id}/rerun` | request caller | preflight on the original version's spec |
| `POST /agents/{id}/runtime` | request caller (manage) | runtime start |
| runtime tick (`background.rs`) | runtime creator, else agent owner | lost access stops the runtime with an error; harness/quota delays the tick |
| scheduled trigger (`background.rs`) | trigger owner, else agent owner | lost access fails the run (audit `trigger.fire_denied`, counts towards auto-disable); harness/quota skip the tick |
| `POST /agent-teams/{id}/runs` | request caller | team **use** + step agents + team connections before the run is created; repository override must be a configured team repository |
| team step start (`start_step`, also advance and queue start) | run requester, else team owner | re-checked for every step; a refused next step fails the run |
| queue start | request caller | same team preflight before the run row exists |

Background identities are loaded with `AccessScope::load_for_user`; disabled users are
refused. Resources with no user owner at all (legacy rows) keep running under platform
authority and only get the harness and quota checks.
