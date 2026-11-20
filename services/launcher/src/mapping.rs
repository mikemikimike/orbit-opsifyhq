//! Pure launch-status mapping over parsed `kubectl get -o json` output and `docker inspect`
//! state. No I/O lives here so every rule can be unit-tested with fixtures.
//!
//! Rules (see `docs/development/kubernetes-launcher.md`):
//! - Status is derived from the CURRENT container state. A past termination
//!   (`lastState`) is only reported as a hint unless the container is not running now.
//! - A container is "OOM" only when the kubelet says `reason == "OOMKilled"`. Exit code 137
//!   alone is SIGKILL (cancel, eviction, `kubectl delete`) and is reported as such.
//! - Waiting reasons that Kubernetes retries forever (CrashLoopBackOff, ImagePullBackOff, ...)
//!   are reported as `failed` with a message instead of staying `starting`.

use serde_json::Value;

/// Result of mapping workload state to an ORBIT launch status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapped {
    pub status: &'static str,
    pub error: Option<String>,
}

impl Mapped {
    fn new(status: &'static str, error: Option<String>) -> Self {
        Self { status, error }
    }
}

/// Waiting reasons that will not resolve without intervention.
const FATAL_WAITING: &[&str] = &[
    "CrashLoopBackOff",
    "ImagePullBackOff",
    "ErrImagePull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
];

fn str_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn memory_limit(pod: &Value) -> &str {
    str_at(pod, "/spec/containers/0/resources/limits/memory").unwrap_or("unknown")
}

fn container_statuses(pod: &Value) -> Vec<&Value> {
    ["/status/containerStatuses", "/status/initContainerStatuses"]
        .into_iter()
        .flat_map(|pointer| {
            pod.pointer(pointer)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .collect()
}

fn oom_message(pod: &Value) -> String {
    format!(
        "worker container was OOMKilled (memory limit {}); increase the worker profile memory limit",
        memory_limit(pod)
    )
}

fn is_oom(terminated: &Value) -> bool {
    terminated.get("reason").and_then(Value::as_str) == Some("OOMKilled")
}

fn describe_termination(terminated: &Value) -> String {
    let reason = str_at(terminated, "/reason").unwrap_or("Error");
    match terminated.get("exitCode").and_then(Value::as_i64) {
        Some(137) => format!(
            "worker container was killed (exit code 137 / SIGKILL, reason {reason}); this is not an out-of-memory kill"
        ),
        Some(code) => format!("worker container exited with code {code} ({reason})"),
        None => format!("worker container terminated ({reason})"),
    }
}

fn termination_message(pod: &Value, terminated: &Value) -> String {
    if is_oom(terminated) {
        oom_message(pod)
    } else {
        describe_termination(terminated)
    }
}

fn restarts(status: &Value) -> i64 {
    status
        .get("restartCount")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// Map the current state of one pod's containers. `restart_always` is true for Deployment
/// pods, where a terminated container is about to be restarted.
fn map_pod(pod: &Value, restart_always: bool) -> Mapped {
    let statuses = container_statuses(pod);
    let mut starting = statuses.is_empty();
    let mut hint: Option<String> = None;
    for status in &statuses {
        if hint.is_none() && status.pointer("/lastState/terminated").is_some_and(is_oom) {
            hint = Some(format!(
                "worker container was OOMKilled earlier (restarts: {}, memory limit {}); consider raising the memory limit",
                restarts(status),
                memory_limit(pod)
            ));
        }
        if let Some(waiting) = status.pointer("/state/waiting") {
            let reason = str_at(waiting, "/reason").unwrap_or("");
            if FATAL_WAITING.contains(&reason) {
                let mut message = format!(
                    "worker container is {reason} (restarts: {})",
                    restarts(status)
                );
                if let Some(last) = status.pointer("/lastState/terminated") {
                    message = format!(
                        "{message}; last termination: {}",
                        termination_message(pod, last)
                    );
                } else if let Some(detail) = str_at(waiting, "/message") {
                    message = format!("{message}: {detail}");
                }
                return Mapped::new("failed", Some(message));
            }
            starting = true;
        } else if let Some(terminated) = status.pointer("/state/terminated") {
            if is_oom(terminated) {
                return Mapped::new("failed", Some(oom_message(pod)));
            }
            if restart_always {
                // Kubernetes is restarting it; this is a transient state.
                starting = true;
            } else if terminated.get("exitCode").and_then(Value::as_i64) == Some(0) {
                return Mapped::new("stopped", None);
            } else {
                return Mapped::new("failed", Some(describe_termination(terminated)));
            }
        } else if status.pointer("/state/running").is_none() {
            starting = true;
        }
    }
    if starting {
        Mapped::new("starting", hint)
    } else {
        Mapped::new("running", hint)
    }
}

fn kind(item: &Value) -> &str {
    item.get("kind").and_then(Value::as_str).unwrap_or("")
}

fn job_condition<'a>(job: &'a Value, condition: &str) -> Option<&'a Value> {
    job.pointer("/status/conditions")
        .and_then(Value::as_array)?
        .iter()
        .find(|c| {
            c.get("type").and_then(Value::as_str) == Some(condition)
                && c.get("status").and_then(Value::as_str) == Some("True")
        })
}

fn job_count(job: &Value, field: &str) -> i64 {
    job.pointer(&format!("/status/{field}"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn map_job(job: &Value, pods: &[&Value]) -> Mapped {
    if job_condition(job, "Complete").is_some() || job_count(job, "succeeded") > 0 {
        return Mapped::new("stopped", None);
    }
    let failed_condition = job_condition(job, "Failed");
    if failed_condition.is_some() || job_count(job, "failed") > 0 {
        let mut parts: Vec<String> = Vec::new();
        match failed_condition {
            Some(condition) => {
                let reason = str_at(condition, "/reason").unwrap_or("Failed");
                match str_at(condition, "/message") {
                    Some(message) => parts.push(format!("job failed ({reason}): {message}")),
                    None => parts.push(format!("job failed ({reason})")),
                }
            }
            None => parts.push("job failed".into()),
        }
        // Add the most specific container-level cause while the pod is still around.
        for pod in pods {
            for status in container_statuses(pod) {
                if let Some(terminated) = status.pointer("/state/terminated") {
                    parts.push(termination_message(pod, terminated));
                }
            }
        }
        return Mapped::new("failed", Some(parts.join("; ")));
    }
    // Still running or starting: a stuck pod (image pull, crash) is a failure.
    let mut best = Mapped::new("starting", None);
    for pod in pods {
        let mapped = map_pod(pod, false);
        match mapped.status {
            "failed" => return mapped,
            "running" | "stopped" => best = mapped,
            _ => {}
        }
    }
    if best.status == "starting" && pods.is_empty() && job_count(job, "active") > 0 {
        return Mapped::new("running", None);
    }
    best
}

/// Map the items of `kubectl get deployments,jobs,pods -l orbit.launch-id=<id> -o json`.
///
/// An empty list means the workload no longer exists (stopped, or a finished Job whose TTL
/// expired). The API persists a terminal status before that happens and only refreshes
/// non-terminal launches, so `stopped` for "gone" never overwrites a stored `failed`.
pub fn map_kubernetes(items: &[Value]) -> Mapped {
    let has_deployment = items.iter().any(|i| kind(i) == "Deployment");
    let job = items.iter().find(|i| kind(i) == "Job");
    let pods: Vec<&Value> = items.iter().filter(|i| kind(i) == "Pod").collect();
    if let Some(job) = job {
        return map_job(job, &pods);
    }
    if has_deployment {
        // Ignore pods that are being replaced.
        let mut best = Mapped::new("starting", None);
        for pod in pods
            .iter()
            .filter(|p| p.pointer("/metadata/deletionTimestamp").is_none())
        {
            let mapped = map_pod(pod, true);
            match mapped.status {
                "failed" => return mapped,
                "running" => best = mapped,
                _ if best.status != "running" => best = mapped,
                _ => {}
            }
        }
        return best;
    }
    if let Some(pod) = pods.first() {
        // Legacy bare Pod launches.
        return match str_at(pod, "/status/phase") {
            Some("Succeeded") => Mapped::new("stopped", None),
            Some("Failed") => {
                let message = container_statuses(pod)
                    .into_iter()
                    .find_map(|s| s.pointer("/state/terminated"))
                    .map(|t| termination_message(pod, t))
                    .or_else(|| str_at(pod, "/status/message").map(str::to_string))
                    .unwrap_or_else(|| "pod failed".into());
                Mapped::new("failed", Some(message))
            }
            _ => map_pod(pod, false),
        };
    }
    Mapped::new("stopped", None)
}

/// Map `docker inspect` state. `oom_killed` comes from `State.OOMKilled`, never from the
/// exit code.
pub fn map_docker(state: &str, oom_killed: bool, exit_code: i64, error: &str) -> Mapped {
    if oom_killed {
        return Mapped::new(
            "failed",
            Some(
                "worker container was OOMKilled (Docker memory limit); increase the worker profile memory limit"
                    .into(),
            ),
        );
    }
    let error = (!error.is_empty()).then(|| error.to_string());
    match state {
        "running" => Mapped::new("running", error),
        "created" | "restarting" => Mapped::new("starting", error),
        "exited" if exit_code == 0 => Mapped::new("stopped", error),
        "exited" => Mapped::new(
            "failed",
            Some(error.unwrap_or_else(|| {
                if exit_code == 137 {
                    "worker container was killed (exit code 137 / SIGKILL); not an out-of-memory kill"
                        .into()
                } else {
                    format!("worker container exited with code {exit_code}")
                }
            })),
        ),
        "removing" => Mapped::new("stopped", error),
        "dead" | "paused" => Mapped::new("failed", error),
        _ => Mapped::new("starting", error),
    }
}

/// `activeDeadlineSeconds` for a single-execution Job: the execution timeout, plus the
/// approval budget (the run's own clock pauses while a person decides, so the wall-clock
/// kill deadline must not), plus a grace window for scheduling, image pull and teardown.
/// Falls back to `default_seconds` when the request has no timeout.
pub fn job_deadline_seconds(
    timeout_seconds: Option<u64>,
    approval_budget_seconds: u64,
    default_seconds: u64,
) -> u64 {
    const GRACE: u64 = 300;
    match timeout_seconds {
        Some(t) if t > 0 => t
            .saturating_add(approval_budget_seconds)
            .saturating_add(GRACE),
        _ => default_seconds.max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pod(container_status: Value) -> Value {
        json!({
            "kind": "Pod",
            "metadata": { "name": "orbit-worker-1234" },
            "spec": { "containers": [{ "resources": { "limits": { "memory": "1Gi" } } }] },
            "status": { "phase": "Running", "containerStatuses": [container_status] }
        })
    }

    fn deployment() -> Value {
        json!({ "kind": "Deployment", "metadata": { "name": "orbit-worker-1234" } })
    }

    fn job(status: Value) -> Value {
        json!({ "kind": "Job", "metadata": { "name": "orbit-worker-1234" }, "status": status })
    }

    #[test]
    fn oomkilled_current_state_is_failed_oom() {
        let items = [
            deployment(),
            pod(json!({
                "restartCount": 0,
                "state": { "terminated": { "reason": "OOMKilled", "exitCode": 137 } }
            })),
        ];
        let mapped = map_kubernetes(&items);
        assert_eq!(mapped.status, "failed");
        let error = mapped.error.unwrap();
        assert!(
            error.contains("OOMKilled") && error.contains("1Gi"),
            "{error}"
        );
    }

    #[test]
    fn sigkill_137_without_oom_reason_is_not_oom() {
        let items = [
            job(json!({ "failed": 1 })),
            pod(json!({
                "state": { "terminated": { "reason": "Error", "exitCode": 137 } }
            })),
        ];
        let mapped = map_kubernetes(&items);
        assert_eq!(mapped.status, "failed");
        let error = mapped.error.unwrap();
        assert!(
            error.contains("SIGKILL") && error.contains("not an out-of-memory"),
            "{error}"
        );
        assert!(!error.contains("increase the worker profile memory limit"));
        // The API keys OOM handling off the word "oom": make sure "not an out-of-memory"
        // does not trip it.
        assert!(!error.to_lowercase().contains("oom"), "{error}");
    }

    #[test]
    fn restarted_after_past_oom_and_now_running_is_running() {
        let items = [
            deployment(),
            pod(json!({
                "ready": true,
                "restartCount": 2,
                "state": { "running": { "startedAt": "2026-11-19T10:00:00Z" } },
                "lastState": { "terminated": { "reason": "OOMKilled", "exitCode": 137 } }
            })),
        ];
        let mapped = map_kubernetes(&items);
        assert_eq!(mapped.status, "running");
        assert!(mapped.error.unwrap().contains("earlier"));
    }

    #[test]
    fn crash_loop_backoff_is_failed_with_restart_count() {
        let items = [
            deployment(),
            pod(json!({
                "restartCount": 5,
                "state": { "waiting": { "reason": "CrashLoopBackOff" } },
                "lastState": { "terminated": { "reason": "Error", "exitCode": 1 } }
            })),
        ];
        let mapped = map_kubernetes(&items);
        assert_eq!(mapped.status, "failed");
        let error = mapped.error.unwrap();
        assert!(
            error.contains("CrashLoopBackOff") && error.contains("restarts: 5"),
            "{error}"
        );
        assert!(error.contains("code 1"), "{error}");
    }

    #[test]
    fn crash_loop_after_oom_keeps_oom_message() {
        let items = [
            deployment(),
            pod(json!({
                "restartCount": 3,
                "state": { "waiting": { "reason": "CrashLoopBackOff" } },
                "lastState": { "terminated": { "reason": "OOMKilled", "exitCode": 137 } }
            })),
        ];
        let mapped = map_kubernetes(&items);
        assert_eq!(mapped.status, "failed");
        assert!(mapped.error.unwrap().contains("OOMKilled"));
    }

    #[test]
    fn image_pull_backoff_is_failed() {
        let items = [
            deployment(),
            pod(json!({
                "restartCount": 0,
                "state": { "waiting": {
                    "reason": "ImagePullBackOff",
                    "message": "Back-off pulling image \"registry/x:1\""
                } }
            })),
        ];
        let mapped = map_kubernetes(&items);
        assert_eq!(mapped.status, "failed");
        assert!(mapped.error.unwrap().contains("ImagePullBackOff"));
    }

    #[test]
    fn container_creating_is_starting() {
        let items = [
            deployment(),
            pod(json!({ "state": { "waiting": { "reason": "ContainerCreating" } } })),
        ];
        assert_eq!(map_kubernetes(&items).status, "starting");
    }

    #[test]
    fn job_failed_carries_condition_message() {
        let items = [job(json!({
            "failed": 1,
            "conditions": [{
                "type": "Failed", "status": "True",
                "reason": "DeadlineExceeded",
                "message": "Job was active longer than specified deadline"
            }]
        }))];
        let mapped = map_kubernetes(&items);
        assert_eq!(mapped.status, "failed");
        let error = mapped.error.unwrap();
        assert!(
            error.contains("DeadlineExceeded") && error.contains("deadline"),
            "{error}"
        );
    }

    #[test]
    fn job_succeeded_is_stopped_without_error() {
        let items = [job(json!({
            "succeeded": 1,
            "conditions": [{ "type": "Complete", "status": "True" }]
        }))];
        assert_eq!(
            map_kubernetes(&items),
            Mapped {
                status: "stopped",
                error: None
            }
        );
    }

    #[test]
    fn job_with_active_running_pod_is_running() {
        let items = [
            job(json!({ "active": 1 })),
            pod(json!({ "ready": true, "state": { "running": {} } })),
        ];
        assert_eq!(map_kubernetes(&items).status, "running");
    }

    #[test]
    fn gone_workload_is_stopped() {
        assert_eq!(map_kubernetes(&[]).status, "stopped");
    }

    #[test]
    fn docker_oom_uses_flag_not_exit_code() {
        assert_eq!(map_docker("exited", true, 137, "").status, "failed");
        let killed = map_docker("exited", false, 137, "");
        assert_eq!(killed.status, "failed");
        assert!(killed.error.unwrap().contains("not an out-of-memory"));
        assert_eq!(map_docker("exited", false, 0, "").status, "stopped");
        assert_eq!(map_docker("running", false, 0, "").status, "running");
    }

    #[test]
    fn job_deadline_derives_from_timeout() {
        assert_eq!(job_deadline_seconds(Some(600), 0, 7200), 900);
        assert_eq!(job_deadline_seconds(None, 3600, 7200), 7200);
        assert_eq!(job_deadline_seconds(Some(0), 3600, 7200), 7200);
    }

    #[test]
    fn job_deadline_includes_the_approval_budget() {
        // timeout 120 s + budget 3600 s + grace 300 s: an approval answered after 8
        // minutes no longer hits DeadlineExceeded.
        assert_eq!(job_deadline_seconds(Some(120), 3600, 7200), 4020);
        assert!(job_deadline_seconds(Some(120), 3600, 7200) > 120 + 8 * 60);
        assert_eq!(job_deadline_seconds(Some(u64::MAX), 3600, 7200), u64::MAX);
    }
}
