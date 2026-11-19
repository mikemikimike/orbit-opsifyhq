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
| Approvals | via execution → agent | — | decision inherits execution | — |
| Dashboard summary | agent/execution/approval/connection/trigger counts, recent executions and usage filtered in SQL (`ResourceAccess`, joined via agent) | — | — | pending approvals via execution → agent |
| Access teams | member list | member | lead/owner/admin | — |
| Sharing | — | view | manage + license | — |

Owner/admin bypass is intentional and documented in ADR 0011.
