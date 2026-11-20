# Agent Teams

An Agent Team coordinates several agents for one outcome. Teams have two modes:

| Mode | How it runs |
|---|---|
| `pipeline` (default) | Every agent is a separate ORBIT execution, chained in order |
| `native` | The primary agent runs once and the remaining agents become harness-native subagents |

## Definition

- Ordered agents with a primary step, per-step instructions and an optional per-step model
  or model override
- Shared instructions, skills and repositories
- Optional **work source**: GitHub/GitLab provider, connection, labels, claim/done/failed
  labels, max items, poll interval and `autoRun`
- Optional queue completion policy

## Queue and dispatcher

The API background dispatcher polls enabled work sources under a per-team advisory lock:

```text
list matching issues → skip items already queued → claim label → insert queue item
→ optionally start the run → release lock
```

Queue items are deduplicated by `(team_id, external_key)`, so two API replicas cannot
double-dispatch. Manual add, run one, run pending, retry/re-queue and cancel are available
in the UI and API (`/api/v1/agent-teams/{id}/queue`). On success the issue gets a comment
with PR/MR links and the done label; failures apply the failed label or release the claim.

## Multi-repository workspace

A team can define `repositories[]` with `connectionId`, `path`, `primary` and `branch`.
Repositories are checked out sequentially (paths are validated against traversal), the
prompt gets a manifest, and git tools accept a `repository` argument. Multi-repo runs
operate on the worker workspace, not the control plane.

## Execution and limits

Team runs are executions (and count against the license's executions/day limit) and use
the same worker profiles, skills, tool policy and approvals as single agents.

## Ownership and visibility

Agent Teams are shareable resources (separate from access teams). They carry
`ownerUserId`, `ownerTeamId` and `visibility` (`private | team | org`). Team lists, team
runs and run detail are filtered by the owning team; starting a run requires view access,
and update/delete require manage access. Org owners/admins govern everything.

## Team connections and environment mappings

A team can attach `connectionIds` and `envMappingIds` directly in the Agent Team editor.
Team connections are validated against visibility and merged into every step's grant set
(capabilities come from the connection scopes, or read-only manifest defaults). Team
environment mappings are resolved into the execution environment for every step. This
avoids duplicating the same GitLab/cloud integration on each step agent; credentials stay
in the broker and are never passed to the harness.

## Auto-approve

Starting a run with `autoApprove: true` (the "Auto-approve writes" checkbox) requires the
`approvals:decide` permission; without it the API returns 403. The API then marks each
step execution with a server-set `approvalOverride: "auto"`. The worker applies it only to
step agents whose own autonomy level is Prepare (L3) or higher, and only by switching the
approval mode to `Auto`. It never raises the level: Observe (L1) and Recommend (L2) agents
still deny every write. Note that at L3 `Auto` mode still requires approval for writes
(only L4 runs them unattended), so in practice the override lifts `Always`/`Writes`
approval modes on L4 agents. The legacy numeric `autonomyOverride` is ignored. Rerunning a
step execution drops the override unless the rerunning user also holds `approvals:decide`.
