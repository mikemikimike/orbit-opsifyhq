//! Opt-in live cluster check. Run with:
//!   ORBIT_TEST_KUBECONFIG="$(kubectl config view --raw --minify)" \
//!     cargo test -p orbit-kubernetes --test live -- --ignored

#[tokio::test]
#[ignore = "needs ORBIT_TEST_KUBECONFIG pointing at a reachable cluster"]
async fn client_certificate_kubeconfig_lists_namespaces() {
    let Some(raw) = std::env::var("ORBIT_TEST_KUBECONFIG").ok() else {
        return;
    };
    let config = serde_json::json!({ "authMode": "kubeconfig" });
    let client = orbit_kubernetes::KubernetesClientConfig::from_connection(&config, Some(raw))
        .expect("kubeconfig parses");
    let result = client
        .test_connection()
        .await
        .expect("client-certificate kubeconfig can reach the API");
    assert!(
        result["count"].as_u64().unwrap_or(0) >= 1,
        "expected at least one visible namespace: {result}"
    );
}

#[tokio::test]
#[ignore = "needs ORBIT_TEST_KUBECONFIG and ORBIT_TEST_KUBECTL_CONTEXT"]
async fn namespace_scan_finds_failed_pod_warning_event_and_logs() {
    let (Some(raw), Some(context)) = (
        std::env::var("ORBIT_TEST_KUBECONFIG").ok(),
        std::env::var("ORBIT_TEST_KUBECTL_CONTEXT").ok(),
    ) else {
        return;
    };
    let namespace = format!("orbit-e2e-{}", std::process::id());
    let kubectl = |args: &[&str]| {
        let status = std::process::Command::new("kubectl")
            .args(["--context", &context, "-n", &namespace])
            .args(args)
            .status()
            .expect("kubectl runs");
        assert!(status.success(), "kubectl {:?}", args);
    };
    let create = std::process::Command::new("kubectl")
        .args(["--context", &context, "create", "namespace", &namespace])
        .status()
        .expect("kubectl runs");
    assert!(create.success());
    kubectl(&[
        "run",
        "crash",
        "--image=busybox:1.36",
        "--restart=Never",
        "--",
        "sh",
        "-c",
        "echo boom; exit 1",
    ]);
    for _ in 0..30 {
        let phase = std::process::Command::new("kubectl")
            .args([
                "--context",
                &context,
                "-n",
                &namespace,
                "get",
                "pod",
                "crash",
                "-o",
                "jsonpath={.status.phase}",
            ])
            .output()
            .expect("kubectl runs");
        if String::from_utf8_lossy(&phase.stdout) == "Failed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    let config = serde_json::json!({
        "authMode": "kubeconfig",
        "namespaces": [namespace],
        "defaultNamespace": namespace,
        "allowedResources": ["pods", "pods/log", "events"]
    });
    let client = orbit_kubernetes::KubernetesClientConfig::from_connection(&config, Some(raw))
        .expect("kubeconfig parses");
    let pods = client
        .invoke("pods.list", &serde_json::json!({}))
        .await
        .expect("pods list");
    assert!(pods["items"]
        .as_array()
        .is_some_and(|items| !items.is_empty()));
    let events = client
        .invoke("events.list", &serde_json::json!({}))
        .await
        .expect("events list");
    assert!(events["items"]
        .as_array()
        .is_some_and(|items| !items.is_empty()));
    let logs = client
        .invoke("pods.logs", &serde_json::json!({ "name": "crash" }))
        .await
        .expect("pod logs");
    assert!(logs["logs"].as_str().unwrap_or("").contains("boom"));

    let _ = std::process::Command::new("kubectl")
        .args([
            "--context",
            &context,
            "delete",
            "namespace",
            &namespace,
            "--wait=false",
        ])
        .status();
}
