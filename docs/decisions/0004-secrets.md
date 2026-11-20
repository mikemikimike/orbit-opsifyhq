# ADR-0004: Pluggable secret store, encrypted at rest

**Status:** Accepted (Phase 0)

## Context
ORBIT must hold provider keys, connector credentials and webhook secrets without ever
exposing them to the frontend or plaintext database dumps.

## Decision
- `SecretStore` trait: `put/get/delete/list_refs`, addressed by opaque `secret_ref`.
- Default backend: PostgreSQL `secrets` table with AES-256-GCM ciphertext, random
  96-bit nonce, AAD = secret ref; master key from `ORBIT_MASTER_KEY` (base64, 32 bytes).
- A future `vault` backend implements the same trait; Helm supports mounting Vault
  agent-injected files and `VAULT_ADDR`/`VAULT_TOKEN` env integration as a Phase 2 item.
- API rules: secrets are write-only; reads return redacted descriptors. Secret values
  are never emitted in logs, execution events, or API payloads.
- Development mode falls back to a deterministic, documented, non-production master key
  when `ORBIT_MASTER_KEY` is unset (logged with a warning). In production the API refuses
  to start when the key is unset or equals either published development key (the base64
  dev value or its SHA-256-derived fallback); the Helm chart rejects the published dev key
  and requires a random key or `secrets.existingSecret`.

## Consequences
- Losing the master key makes stored credentials unrecoverable (documented).
- Key rotation is not implemented yet; the ciphertext format includes a `v1` prefix.
