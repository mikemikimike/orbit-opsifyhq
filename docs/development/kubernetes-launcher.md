# Kubernetes worker launch authentication

## Overview

The `orbit-launcher` service is the only component with cluster privileges. It shells
out to `kubectl` to create and manage worker pods. The API never receives kubectl
or Docker socket access — it stores launch intent and calls the launcher over a
separate authenticated HTTP channel.

The launcher is the default way platform workers run (`launcher.enabled=true` in the Helm
chart). The API mints a scoped worker token per launch, hands it to the launcher, and
revokes it when the launch stops. Operators do not issue worker tokens for platform
workers; tokens exist for custom/external workers only.

## How authentication works

### 1. Kubeconfig (default)

The launcher container mounts a kubeconfig file:

```yaml
volumeMounts:
  - name: kubeconfig
    mountPath: /home/orbit/.kube
```

`kubectl` automatically picks up `~/.kube/config` (or the `KUBECONFIG` env var).

### 2. In-cluster service account (recommended for production)

If the launcher runs inside the cluster, it uses the pod's mounted service account
token automatically:

```
/var/run/secrets/kubernetes.io/serviceaccount/token
```

No kubeconfig needed — kubectl detects the in-cluster environment.

## RBAC requirements

The launcher needs these permissions in the target namespace:

```yaml
rules:
- apiGroups: [""]
  resources: ["pods"]
  verbs: ["get", "list", "watch", "create", "delete"]
- apiGroups: [""]
  resources: ["pods/log"]
  verbs: ["get"]
- apiGroups: ["apps"]
  resources: ["deployments"]
  verbs: ["get", "list", "watch", "create", "delete"]
- apiGroups: ["batch"]
  resources: ["jobs"]
  verbs: ["get", "list", "watch", "create", "delete"]
```

The Helm chart creates a dedicated `ServiceAccount` and `Role`/`RoleBinding` for this.

## Image pull secrets

If your registry requires authentication:

```yaml
ORBIT_LAUNCHER_IMAGE_PULL_SECRET=registry-credentials
```

The launcher adds `imagePullSecrets` to every pod it creates.

## Key environment variables

| Variable | Purpose |
|---|---|
| `ORBIT_LAUNCHER_KUBECTL` | Path to kubectl (default `kubectl`) |
| `ORBIT_LAUNCHER_NAMESPACE` | Target namespace for worker pods (default `orbit`) |
| `ORBIT_LAUNCHER_IMAGE_PULL_SECRET` | Secret name for registry auth |
| `ORBIT_LAUNCHER_IMAGE_ALIASES` | Rewrite registry hosts (e.g. `registry.example.com:5000=registry.example.com:5000`) |
| `ORBIT_LAUNCHER_TOKEN` | Bearer token the API uses to authenticate to the launcher. The Helm chart generates it and reuses it from the release Secret when `launcher.token` is empty; set the value explicitly for GitOps. |

## Security model

- The API stores launch intent and calls `POST /launches` on the launcher with a
  bearer token (`ORBIT_LAUNCHER_TOKEN`).
- The launcher validates the token, then runs `kubectl create -f -` with the workload
  manifest (Deployment for long-lived profiles, Job for single executions).
- Worker pods receive `ORBIT_WORKER_TOKEN` (a scoped worker token minted for that
  launch) via env var. The token is revoked when the launch stops or fails.
- The launcher never exposes kubectl or the Docker socket to the API or web app.

## Custom CA certificates

When `caCertificates.enabled=true`, the chart passes the CA Secret name/key to the API
(`ORBIT_CA_SECRET_NAME`, `ORBIT_CA_SECRET_KEY`). The launcher mounts that Secret into every
worker Pod/Job at `/etc/ssl/certs/ca-bundle.crt` and sets `SSL_CERT_FILE`,
`REQUESTS_CA_BUNDLE` and `NODE_EXTRA_CA_CERTS`. This covers harnesses that use the system
store (Rust workers) and Node/Bun-based harnesses (OpenCode).

The worker pod needs read access to the Secret; the launcher itself does not read it.
