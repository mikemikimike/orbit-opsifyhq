# ORBIT Architecture Overview

ORBIT is a self-hosted control plane for creating, connecting, deploying and governing
persistent AI agents. It is **not** an agent framework: agent reasoning happens in
dedicated harness processes (OpenCode, Claude Code, Pi, custom) behind a stable adapter
contract.

## 1. Layers

```
┌──────────────────────────────────────────────────────────────────────┐
│ A. Agent Experience                                                  │
│    Next.js web app (apps/web)  ── future Expo/React Native apps      │
└───────────────▲──────────────────────────────────────────────────────┘
                │ REST /api/v1 + SSE (OpenAPI contract)
┌───────────────┴──────────────────────────────────────────────────────┐
│ B. Control Plane                                                     │
│    orbit-api (axum)  ── authn/z, agents, executions, policies, audit  │
│    orbit-worker      ── event routing, harness dispatch, tracking     │
│    PostgreSQL        ── durable state                                 │
│    NATS JetStream    ── durable events (dev: in-memory bus)           │
└───────────────▲──────────────────────────────────────────────────────┘
                │ HarnessAdapter contract (crates/orbit-contracts)
┌───────────────┴──────────────────────────────────────────────────────┐
│ C. Execution Plane                                                   │
│    Isolated harness workers / containers / Kubernetes Jobs            │
│    opencode · pi · mock (available) · claude-code / generic (planned) │
└──────────────────────────────────────────────────────────────────────┘
```

The control plane never embeds an agent loop. The worker owns adapter instances; in
local development the mock harness runs in-process in `orbit-worker`, and production
harnesses are launched as isolated containers/Jobs (`HarnessDispatcher`).

## 2. Harness topology (MVP)

| Component | Local (Compose) | Kubernetes (Helm) |
|---|---|---|
| `orbit-api` | container, port 8080 | Deployment + Service + Ingress |
| `orbit-worker` | container | Deployment (mock) or Job per execution |
| `orbit-worker-opencode` | container | Deployment (OpenCode CLI bundled) |
| `orbit-worker-pi` | container | Deployment (Pi CLI bundled) |
| web | container, port 3000 | Deployment + Service + Ingress |
| PostgreSQL | container | external or bundled (opt-in) |
| NATS JetStream | container | external or bundled (opt-in) |
| Harness processes | mock in-process; OpenCode/Pi as subprocess in a per-execution workspace | Pod/Job per worker; Job per single execution via the launcher |

## 3. Code structure

ORBIT is a Rust workspace with a Next.js frontend:

| Directory | Purpose |
|---|---|
| `crates/orbit-domain` | Domain types, validation, autonomy policy |
| `crates/orbit-contracts` | Cross-boundary traits (harness, provider, connector) |
| `crates/orbit-store` | PostgreSQL repositories and migrations |
| `crates/orbit-api` | HTTP API (handlers, routes, OpenAPI) |
| `crates/orbit-harness` | Harness adapters (mock, OpenCode, Pi) |
| `services/api` | Control-plane binary |
| `services/worker` | Execution-plane binary |
| `apps/web` | Next.js web app |

See `AGENTS.md` in the repository root for the full repository map.

## 4. Request & execution flow

Manual execution:

1. `POST /api/v1/agents/{id}/executions` validates agent state, autonomy and quotas.
2. A row in `executions` is inserted with status `queued` and an idempotency key.
3. `orbit.execution.requested` is published on the event bus.
4. The worker claims the execution (`queued → dispatched`, compare-and-set), resolves
   the harness adapter, creates/resumes a session and streams events.
5. Events are persisted in `execution_events`; execution status transitions are
   persisted and published as `orbit.execution.updated`.
6. The frontend stream (`GET /api/v1/executions/{id}/events/stream`) replays persisted
   events and then tails new ones (SSE, `Last-Event-ID` supported).

Scheduled and team-driven executions use the same queued path. Background loops live in
`orbit-api`: the scheduler claims due cron triggers and stores their next run atomically (`FOR UPDATE SKIP LOCKED`), the team
advancer moves pipeline runs forward, the work-source dispatcher enqueues issues, and the
reaper fails orphaned executions. A worker only needs `ORBIT_API_URL` and a scoped
`ORBIT_WORKER_TOKEN` — no database, bus or master-key access. Retention and deduplication
live in PostgreSQL (`processed_events`, unique `idempotency_key`), not in memory.

## 5. Isolation & security

- Agent tasks are **never** executed inside `orbit-api`.
- Adapters receive a constrained `HarnessExecutionContext` (timeout, CPU/memory,
  network policy (recorded, not yet enforced), filesystem policy, injected secret handles — never raw secrets in
  prompts or logs).
- Autonomy is enforced by the platform before dispatch and at capability invocation
  time: connector capabilities are checked against per-agent allowlists and autonomy
  level; level ≤ 2 is read-only, level 3 creates approvals, level 4 executes allowed
  capabilities.
- All secret material is encrypted at rest (AES-256-GCM, `ORBIT_MASTER_KEY`) and is
  never returned by the API. `SecretStore` is pluggable (Vault later).
- External content is untrusted: harness input is wrapped with provenance metadata and
  trusted system instructions are transported separately.

## 6. Extensibility

- **New harness** → implement `HarnessAdapter` in `crates/orbit-harness`, register it in
  the registry; no domain/API changes.
- **New provider kind** → add a `ProviderKind` variant with a config struct and
  capability set; harness adapters translate it to their own provider format.
- **New connector** → add a manifest in `crates/orbit-api/src/connectors.rs` and a
  client in the worker; the API and UI read the registry automatically.
- **New infrastructure** → Compose and Helm consume the same env-var contract.

## 7. Mobile readiness

The backend is a pure HTTP/JSON + SSE API with bearer-token auth (OIDC access tokens,
`orbit_sk_…` API tokens and separate `orbit_wk_…` worker tokens), versioned under
`/api/v1`, with keyset pagination and a stable error envelope. No cookie-only or browser-specific behavior is required, so an
Expo/React Native client can share `packages/api-client` and `packages/contracts`.

## 8. Observability

- Structured JSON logs (`tracing`) with request IDs and trace IDs.
- `GET /healthz` (liveness) and `GET /readyz` (readiness depends on the database; event bus health is reported but non-gating).
- `GET /metrics` Prometheus text endpoint (core counters; OpenTelemetry export seam).
- Execution events carry `execution_id`, `agent_id`, `harness`, and timestamps for
  correlation across control and execution planes.
