# Kubernetes connector

Read-only cluster inspection for agents: list namespaces/pods/deployments/events, read pod
details and bounded logs. No writes, exec, port-forward or Secret access.

## Connection

Add a **Kubernetes** connection. Auth modes:

- `service_account` (preferred): paste a reader ServiceAccount token as the credential and
  set `apiServer` (in-cluster: `https://kubernetes.default.svc`).
- `kubeconfig`: paste a kubeconfig as the credential. ORBIT uses the selected/current
  context and supports static `token` or `client-certificate-data` +
  `client-key-data` auth. It **rejects** `exec` auth plugins and
  `insecure-skip-tls-verify`.

Config fields: `namespaces` allowlist, `defaultNamespace`, `allowedResources`
(`pods`, `pods/log`, `deployments`, `events`, `namespaces`), optional public `caCert`,
`timeoutSeconds`, `maxLogBytes`. Allowlists are enforced by ORBIT as well as RBAC.

## Test connection

The connection **Test** button parses the credential, connects to the API server and lists
the namespaces visible to the credential. Unsupported kubeconfig auth, rejected TLS
settings and RBAC failures return a readable message; the result is stored as the
connection check status.

The same parser and client are shared by the API (test) and worker (tool calls), so a
connection that tests successfully behaves the same during an execution.

## Capabilities

`namespaces.list`, `pods.list`, `pods.get`, `pods.logs` (tail/previous, capped),
`deployments.list`, `deployments.get`, `events.list`.

## RBAC example

See `deploy/examples/kubernetes-reader-rbac.yaml` in the repository.

Do not grant Secrets, pods/exec, port-forward, create/delete or cluster-wide access.

## Verification

`crates/orbit-kubernetes/tests/live.rs` has opt-in tests for a real cluster:

```bash
ORBIT_TEST_KUBECONFIG="$(kubectl config view --raw --minify --context kind-orbit)" \
  ORBIT_TEST_KUBECTL_CONTEXT=kind-orbit \
  cargo test -p orbit-kubernetes --test live -- --ignored
```

The second test creates a failing pod in a temporary namespace and verifies pods, warning
events and pod logs through the same client the worker uses. Verified on kind with
client-certificate kubeconfig auth.
