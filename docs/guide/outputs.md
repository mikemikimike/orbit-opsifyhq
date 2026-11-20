# Outputs (webhooks, Slack, Mattermost)

Outputs deliver an execution result after it reaches a terminal state. Manage them under
**Outputs** in the sidebar (or via the API below).

## UI

The Work wizard's **Then** step lists outputs configured for the selected agent/team before
the run is created.

**Outputs → New output**: name, kind, optional agent, events, incoming webhook URL,
optional template and enabled toggle. **Test** sends a sample notification; edit leaves the
URL blank to keep the existing one.

## Create via API

```bash
curl -X POST "$API/api/v1/outputs" \
  -H "Authorization: Bearer $ORBIT_TOKEN" \
  -H 'content-type: application/json' \
  -d '{
    "name": "release-slack",
    "kind": "slack",
    "agentId": null,
    "events": ["succeeded", "failed"],
    "webhookUrl": "https://hooks.slack.com/services/...",
    "template": "Release finished"
  }'
```

- `kind`: `slack`, `mattermost` or `webhook`
- `agentId`: one agent, or `null` together with `teamId` for all executions
- `teamId`: one Agent Team; fires once when the team run finishes (not per child execution)
- `events`: `succeeded`, `failed`, `cancelled` (default: all three)
- `webhookUrl`: write-only; encrypted at rest and never returned

For Slack/Mattermost use an **incoming webhook URL**. The body is `{"text": "..."}`.

For `webhook`, ORBIT POSTs JSON with `event`, `text` and an `execution` object.

## Test

```bash
curl -X POST "$API/api/v1/outputs/$OUTPUT_ID/test" \
  -H "Authorization: Bearer $ORBIT_TOKEN"
```

## Delivery

Delivery is **at-least-once** and driven by a background outbox loop on every API replica.

- **Enqueue**: each output keeps a high-water mark and only delivers runs that finished
  after it was created (or last re-enabled), so a new output never back-fills history and an
  API outage of any length still delivers every finished run once. Deliveries are unique per
  `(output, execution)` / `(output, team run)`.
- **Claim**: rows are claimed atomically (`FOR UPDATE SKIP LOCKED`) with a 120 s lease, so
  replicas and overlapping ticks never send the same delivery concurrently. A sender that
  crashes mid-send leaves the row `sending`; it is reclaimed after the lease expires and
  counts as an attempt.
- **Idempotency**: every delivery sends its id as `Idempotency-Key` and
  `X-Orbit-Delivery-Id` headers (and `deliveryId` in `webhook` bodies). Because a crash
  after the receiver processed the request but before ORBIT recorded it causes a resend,
  receivers should de-duplicate on this id. Slack/Mattermost ignore the headers.
- **Failures**:

| Result | Action |
|---|---|
| 2xx | `succeeded` |
| 5xx, 408, 429, network error/timeout | retry with backoff (30 s x attempt); `dead` after 5 attempts |
| other 4xx / 3xx, missing webhook secret | `failed` immediately, never retried |
| output disabled or deleted, run deleted | `skipped` (no request sent) |

- List outputs: `GET /api/v1/outputs`; update/delete via `PATCH/DELETE /api/v1/outputs/{id}`.

Not yet implemented: HMAC signing for generic webhooks, SSRF/private-network blocking,
team scoping, a delivery-log UI, and template rendering beyond the built-in summary.
