# Domain Model

## Aggregate relationships

```
organization 1─* user
organization 1─* agent 1─* agent_version
agent        1─* agent_instance *─1 harness
agent        1─* trigger
agent        1─* execution *─1 agent_version
execution    1─* execution_event
execution    1─* approval
organization 1─* connection *─1 connector
organization 1─* provider
organization 1─* policy
organization 1─* audit_log
organization 1─* agent_team 1─* team_run 1─* team_step *─1 execution
agent_team   1─* team_queue_item *─1 team_run
organization 1─* skill
organization 1─* worker_profile 1─* worker_launch
organization 1─* worker_token
organization 1─* api_token
organization 1─* secret
agent        1─* agent_memory
```

## Tables

Every table is scoped by `org_id` unless it is global infrastructure.

| Table | Entity |
|---|---|
| `organizations`, `users` | tenant and members |
| `agents`, `agent_versions`, `agent_instances`, `harnesses` | agent definitions, immutable versions, long-running instances, harness registry |
| `providers`, `connectors`, `connections`, `secrets` | model providers, connector manifests, authorized accounts, encrypted secrets |
| `executions`, `execution_events`, `approvals` | runs, append-only events, human decisions |
| `triggers`, `policies`, `processed_events` | scheduled/event triggers, policy documents, bus idempotency ledger |
| `agent_teams`, `team_runs`, `team_steps`, `team_queue_items` | Agent Teams, runs, steps, durable work queue |
| `agent_memory` | short-term/summary agent memory |
| `skills` | reusable SKILL.md packages |
| `worker_profiles`, `worker_tokens`, `worker_launches`, `worker_heartbeats` | execution-plane configuration, credentials, instances, liveness |
| `api_tokens`, `audit_logs`, `platform_settings` | user tokens, audit trail, platform configuration |

Migrations live in `crates/orbit-store/migrations` and are append-only.

## Entities

### Organization
Tenant boundary. `mode = personal | enterprise`. A personal organization is created
automatically for each user in self-hosted deployments. All queries are scoped by
`org_id`; enterprise RBAC builds on `users.role` and `policies`.

### User
`role ∈ {owner, admin, operator, viewer}`. Links to an external OIDC identity via
`(issuer, subject)`; never stores passwords.

### Agent (persistent definition)
| Field | Notes |
|---|---|
| `id`, `org_id`, `slug`, `name`, `description` | `slug` unique per org |
| `status` | `draft → active ⇄ paused → archived` |
| `current_version` | integer, points at an immutable `agent_versions` row |
| `labels` | JSONB for filtering |

A configuration change creates a new immutable version. Executions always reference the
exact `agent_version_id` they ran with.

### AgentVersion
`spec` JSONB is a validated `AgentSpec` (`apiVersion: orbit.io/v1`, `kind: Agent`).
`checksum` is the SHA-256 of canonical JSON; duplicate specs are rejected per agent.
`definition` retains the original YAML for export.

### AgentInstance
A deployed representation: `agent_version_id`, `harness_id`, `state`
(`pending → starting → running → stopping → stopped → failed`), `harness_instance_ref`,
`health`, harness-specific `config`. On-demand harnesses may not need instances; the
model supports both long-running and ephemeral execution.

### Harness (registration)
`kind ∈ {mock, opencode, claude_code, pi, generic}`. Carries a capability document
(`supports_sessions`, `supports_resume`, `supports_streaming`, `supports_cancel`,
`supports_instances`, `supports_tool_events`, `supports_usage_reporting`,
`harness_managed_providers`, `supports_platform_providers`, `execution_modes`,
`max_timeout_seconds`, `max_memory_mb`). Health is reported by workers via heartbeats;
the control plane never probes harness internals directly.

### Provider
Model provider configuration **separate from harnesses**. `kind ∈
{openai_compatible, harness_managed}`. Secrets are `secret_ref` handles resolved by the
backend `SecretStore`; API responses redact them (`has_credential: true`).

### Connector / Connection / Capability / Policy
- **Connector**: integration definition (slug, version, capability catalog, auth kinds).
- **Connection**: authorized account for a connector in an org, with `credential_ref`,
  scopes and status (`active | expired | revoked | error`).
- **Capability**: `resource.action` (e.g. `issues.create`), classified by effect
  (`read | write | external_side_effect`).
- **Policy**: JSON document scoped to org/agent/connection; evaluated by the platform,
  not by the model.

### Trigger
`kind ∈ {manual, schedule, webhook, event}` (enforced by a CHECK constraint). Config is
validated per kind: a 5-field cron expression plus an optional IANA `timezone` (default
UTC; unknown zones are rejected) for `schedule`, a subject filter for
`event`, an HMAC secret reference for `webhook`. Triggers can be disabled without being
deleted, and a trigger auto-disables after `max_failures` consecutive failures.
A `schedule` trigger may also carry a `task`: the text given to the agent each time it
fires. Without one the run gets a generic "perform your configured task" prompt.

Schedule semantics: the cron expression is evaluated in the trigger's `timezone`, so
`0 9 * * *` in `Europe/Amsterdam` fires at 09:00 local time across DST changes. The
scheduler claims a due trigger and stores its next `next_run_at` in the same transaction
(missed runs are not replayed), and a sweep repairs any enabled schedule trigger left
with a NULL `next_run_at`. Enabling a trigger recomputes `next_run_at` from now.

Triggers declared in an agent spec are *managed* (`triggers.managed = true`): each new
agent version replaces only managed rows, keeping the id and history of a spec trigger
whose kind and config are unchanged. Triggers created through `POST /api/v1/triggers`
are never modified by version sync.

### Execution
Lifecycle:

```
queued → dispatched → running → succeeded | failed | timed_out | cancelled
                              └─→ awaiting_approval → running | rejected
```

Fields: `trigger_source`, `placement`, `input`, `output`, `error`, `session_ref`,
`usage`, `skills`, `started_at`, `finished_at`, `archived_at`, `team_run_id`,
`team_step_index`, `idempotency_key` (unique). Terminal states are final and never
mutated. Claiming is a SQL compare-and-set (`UPDATE … WHERE status = 'queued'`), so
concurrent workers cannot double-claim. `archived_at` hides old terminal executions
from the default view without deleting history.

### ExecutionEvent
Append-only, per-execution monotonic `seq`: `kind ∈ {log, status, tool_call, output,
error, metric}`, `level`, `message`, `data`. Drives the SSE stream and the UI timeline.

### AuditLog
Append-only records of security-relevant actions (agent changes, secret changes,
connection grants, approvals, executions).

### Agent Team / TeamRun / TeamStep / TeamQueueItem
- **AgentTeam**: `definition` JSONB holds mode (`pipeline` | `native`), instructions,
  ordered steps (agent + optional model override), skills, optional work source,
  multi-repository list, optional worker profile and `autoLaunch`. Tracks
  `last_poll_at` / `last_poll_error`.
- **TeamRun**: one orchestration run; terminal states `succeeded | failed | cancelled`.
  Carries `archived_at`.
- **TeamStep**: one agent step of a run (unique `run_id, step_index`), linked to the
  execution it created.
- **TeamQueueItem**: durable work item, unique per `(team_id, external_key)` so an issue
  is never queued twice. Status `pending | running | succeeded | failed | cancelled`;
  tracks `attempts` / `max_attempts` for retry.

### Skill
Reusable SKILL.md package: `slug`, `content`, JSONB `files` (path → text, including
`scripts/*`), monotonic `version`.

### WorkerProfile / WorkerToken / WorkerLaunch / WorkerHeartbeat
- **WorkerProfile**: pins harness kind, target, execution driver, image + tag, agent
  scope (`all` dynamic or `selected` with an explicit allowlist), skill allowlist, optional
  provider/model, optional auth secret name, static `env`, `env_mappings`
  (connection/secret → env var), typed CPU/memory requests/limits + workspace size
  (`WorkerResources`) and `enabled`. Worker sizing lives here, not on the agent.
- **WorkerToken**: scoped, revocable, hashed worker credential (optionally bound to a
  profile and a single execution).
- **WorkerLaunch**: a launcher-managed instance (`pending | starting | running |
  stopped | failed`) with runtime id, worker id and `archived_at`.
- **WorkerHeartbeat**: live worker registry (harnesses, target, hostname, last seen).

### ApiToken / Secret / PlatformSetting / ProcessedEvent
- **ApiToken**: hashed `orbit_sk_…` token with scopes and revocation.
- **Secret**: AES-256-GCM ciphertext keyed by an opaque `reference`.
- **PlatformSetting**: key/value JSONB for platform config such as OIDC.
- **ProcessedEvent**: idempotency ledger for bus delivery.

### AgentMemory
Short-term or summary memory for an agent, with optional `expires_at`; pruned hourly.

## Invariants

1. Every agent row has at least one immutable version.
2. Executions are never mutated in terminal states.
3. A capability invocation must be allowed by the agent's autonomy level **and** the
   connection's capability allowlist.
4. Secrets are write-only through the API.
5. Cross-organization references are rejected at the store layer.
