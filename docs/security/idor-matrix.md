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
| Worker launches | via worker profile | via worker profile | owner/admin or profile manager | logs via worker profile |
| Worker API executions | — | claiming token only (`404` otherwise) | claiming token only | events/complete/approvals/audit/connection/memory bound to claiming token |
| Worker heartbeats | org-scoped | — | owning token only | — |
| Agent memory (worker API) | — | via claimed execution's agent | via claimed execution's agent (64 KiB cap) | — |
| Access teams | member list | member | lead/owner/admin | — |
| Sharing | — | view | manage + license | — |

Worker-surface rows (#139): a worker token acts only on executions it claimed. The claim
stores the token id; legacy rows claimed before binding (NULL token, worker id set) stay
reachable by org/profile scope until they finish. Tokens shared by several workers (e.g.
one profile token for a pool, or Docker-driver child containers) are one principal.

Owner/admin bypass is intentional and documented in ADR 0011.
