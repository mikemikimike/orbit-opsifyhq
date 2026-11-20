# Kubernetes connector

Read-only cluster inspection for agents: list namespaces/pods/deployments/events, read pod
details and bounded logs. No writes, exec, port-forward or Secret access.

## Connection

Add a **Kubernetes** connection. Auth modes:

- `service_account` (preferred): paste a reader ServiceAccount token as the credential and
  set `apiServer` (in-cluster: `https://kubernetes.default.svc`).
- `kubeconfig`: paste a kubeconfig as the credential. ORBIT uses the selected/current
  context and supports static `token` auth only. It **rejects** `exec` auth plugins,
  client-certificate auth for now, and `insecure-skip-tls-verify`.

Config fields: `namespaces` allowlist, `defaultNamespace`, `allowedResources`
(`pods`, `pods/log`, `deployments`, `events`, `namespaces`), optional public `caCert`,
`timeoutSeconds`, `maxLogBytes`. Allowlists are enforced by ORBIT as well as RBAC.

## Capabilities

`namespaces.list`, `pods.list`, `pods.get`, `pods.logs` (tail/previous, capped),
`deployments.list`, `deployments.get`, `events.list`.

## RBAC example

See `deploy/examples/kubernetes-reader-rbac.yaml` in the repository.

Do not grant Secrets, pods/exec, port-forward, create/delete or cluster-wide access.
