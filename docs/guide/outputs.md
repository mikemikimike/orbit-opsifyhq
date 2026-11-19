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

Webhook URLs are checked against the outbound network policy (loopback, link-local and
cloud-metadata targets are refused; see [Providers](providers.md#outbound-network-policy)).

## Delivery

- A background outbox loop enqueues terminal executions and retries failed deliveries with
  backoff (5 attempts, then `dead`).
- Deliveries are unique per `(output, execution)`.
- List outputs: `GET /api/v1/outputs`; update/delete via `PATCH/DELETE /api/v1/outputs/{id}`.

Not yet implemented: HMAC signing for generic webhooks, SSRF/private-network blocking,
team scoping, a delivery-log UI, and template rendering beyond the built-in summary.
