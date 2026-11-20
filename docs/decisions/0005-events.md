# ADR-0005: Event bus abstraction; JetStream in production, memory in dev

**Status:** Accepted (Phase 0 foundation)

## Context
Execution and trigger flows are event-driven and must survive restarts. Local
development should not require operating Kafka/Redis; the requirement names NATS
JetStream.

## Decision
- `EventBus` trait with `memory` and `nats` drivers selected by `ORBIT_EVENTS_DRIVER`.
- NATS uses a single JetStream stream (`ORBIT`) with subject `orbit.>`; workers use
  durable pull consumers with explicit ack and bounded redelivery.
- Durable state lives in PostgreSQL (`executions`, `execution_events`,
  `processed_events`), never in the bus or process memory.
- Handlers are idempotent: dedupe table + SQL compare-and-set claiming.
- No Kafka, Redis or Temporal in the MVP.

## Update (v0.21.x, #125)
Durable pull consumers with explicit ack and bounded redelivery were never implemented;
the NATS driver publishes to the `ORBIT` stream (the JetStream ack is now confirmed in the background and logged on failure) and
subscribes with plain core subscriptions. The bus is an accelerator only: PostgreSQL stays
the source of truth, and `/readyz` no longer depends on bus health. The text above records
the original intent.

## Consequences
- Local dev and tests need no external broker; production gets at-least-once delivery.
- The memory driver is explicitly non-durable and refuses to start when
  `ORBIT_ENV=production`.
