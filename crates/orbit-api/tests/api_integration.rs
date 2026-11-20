use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use orbit_api::{build_router, AppState, DevIdentity};
use orbit_domain::UserRole;
use orbit_events::{EventBusError, MemoryEventBus};
use orbit_harness::HarnessRegistry;
use orbit_security::{AuthConfig, AuthMode, DevAuthConfig, SecretCipher};
use orbit_store::Store;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

/// Event bus whose health can be flipped to simulate a NATS outage.
struct SwitchableBus {
    inner: MemoryEventBus,
    healthy: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl orbit_events::EventBus for SwitchableBus {
    fn driver(&self) -> &'static str {
        "memory"
    }
    async fn publish(&self, event: orbit_contracts::OrbitEvent) -> Result<(), EventBusError> {
        self.inner.publish(event).await
    }
    async fn subscribe(
        &self,
        filter: orbit_contracts::SubjectFilter,
    ) -> Result<orbit_events::EventStream, EventBusError> {
        self.inner.subscribe(filter).await
    }
    async fn health(&self) -> Result<(), EventBusError> {
        if self.healthy.load(std::sync::atomic::Ordering::SeqCst) {
            Ok(())
        } else {
            Err(EventBusError::Unavailable("simulated outage".into()))
        }
    }
}

struct TestApp {
    bus_healthy: Arc<std::sync::atomic::AtomicBool>,
    shutdown: tokio_util::sync::CancellationToken,
    router: axum::Router,
    state: AppState,
    store: Store,
    org_id: Uuid,
    agent_id: Uuid,
}

async fn test_app(allow_anonymous: bool) -> Option<TestApp> {
    let url = std::env::var("ORBIT_TEST_DATABASE_URL").ok()?;
    let key = base64::engine::general_purpose::STANDARD.encode([9u8; 32]);
    let cipher = Arc::new(SecretCipher::from_base64_key(&key).unwrap());
    let store = Store::connect(&url, 5)
        .await
        .expect("connect")
        .with_cipher(cipher);
    store.migrate().await.expect("migrate");

    let slug = format!("api-test-{}", Uuid::now_v7());
    let org = store
        .create_organization(&slug, "API Test", orbit_domain::OrganizationMode::Personal)
        .await
        .unwrap();
    let user = store
        .ensure_user(org.id, "api@test.local", "API Test", UserRole::Owner)
        .await
        .unwrap();

    // Seed one connector so connection endpoints work.
    for manifest in orbit_api::connectors::builtin_manifests() {
        store.upsert_connector_manifest(&manifest).await.unwrap();
    }
    store
        .upsert_harness_descriptor(
            &HarnessRegistry::builtin()
                .descriptors()
                .into_iter()
                .find(|d| d.kind == orbit_domain::HarnessKind::Mock)
                .unwrap(),
        )
        .await
        .unwrap();

    let auth_config = AuthConfig {
        mode: AuthMode::Dev,
        dev: DevAuthConfig {
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            org_slug: org.slug.clone(),
            org_name: org.name.clone(),
            role: UserRole::Owner,
            allow_anonymous: true,
            dev_token: "test-token".into(),
        },
        oidc: None,
        allow_api_tokens: true,
    };

    let bus_healthy = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let shutdown = tokio_util::sync::CancellationToken::new();
    let bus = Arc::new(SwitchableBus {
        inner: MemoryEventBus::new(128),
        healthy: bus_healthy.clone(),
    });
    let state = AppState {
        store: store.clone(),
        bus,
        registry: Arc::new(HarnessRegistry::builtin()),
        auth: Arc::new(auth_config),
        jwt: None,
        default_org_id: org.id,
        dev_identity: Some(Arc::new(DevIdentity {
            user_id: user.id,
            org_id: org.id,
            email: user.email.clone(),
            display_name: user.display_name.clone(),
            role: UserRole::Owner,
        })),
        environment: "test".into(),
        events_driver: "memory".into(),
        public_base_url: "http://localhost".into(),
        metrics: Arc::new(orbit_api::metrics::Metrics::new()),
        max_body_bytes: 2 * 1024 * 1024,
        cors_origins: vec!["*".into()],
        launcher_url: None,
        launcher_token: None,
        worker_api_url: "http://localhost:8080".into(),
        ca_secret_name: None,
        ca_secret_key: "ca-bundle.crt".into(),
        archive_after_days: None,
        shutdown: shutdown.clone(),
        license: orbit_security::LicenseHandle::load(orbit_security::LicenseSource {
            token: None,
            file: None,
        }),
    };
    // In this test harness anonymous is controlled by auth_config.dev.allow_anonymous;
    // rebuild the config accordingly for the negative auth test.
    let mut state = state;
    let mut auth = (*state.auth).clone();
    auth.dev.allow_anonymous = allow_anonymous;
    state.auth = Arc::new(auth);

    let created = create_demo_agent(&store, org.id, user.id).await;
    Some(TestApp {
        bus_healthy,
        shutdown,
        router: build_router(state.clone()),
        state,
        store,
        org_id: org.id,
        agent_id: created,
    })
}

fn definition() -> Value {
    json!({
        "apiVersion": "orbit.io/v1",
        "kind": "Agent",
        "metadata": { "name": "API Test Agent", "description": "created by tests" },
        "spec": {
            "harness": { "type": "mock", "executionMode": "on_demand", "config": {} },
            "model": { "management": "harness" },
            "instructions": { "system": "You are a test agent." },
            "connectors": [],
            "triggers": [{ "type": "schedule", "cron": "0 * * * *" }],
            "autonomy": { "level": 2 },
            "resources": { "timeoutSeconds": 30, "maxConcurrentExecutions": 1 }
        }
    })
}

async fn create_demo_agent(store: &Store, org_id: Uuid, user_id: Uuid) -> Uuid {
    let definition: orbit_domain::AgentDefinition = serde_json::from_value(definition()).unwrap();
    let (agent, _) = store
        .create_agent(org_id, Some(user_id), &definition)
        .await
        .unwrap();
    store
        .set_agent_status(org_id, agent.id, orbit_domain::AgentStatus::Active)
        .await
        .unwrap();
    agent.id
}

async fn send(router: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

fn post(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn public_endpoints_and_meta() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let (status, body) = send(&app.router, get("/healthz")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");

    let (status, body) = send(&app.router, get("/api/v1/meta")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["apiVersion"], "v1");
    assert_eq!(body["auth"]["mode"], "dev");

    let (status, body) = send(&app.router, get("/api/v1/openapi.json")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["paths"]["/api/v1/agents"].is_object());
}

#[tokio::test]
async fn readyz_depends_on_database_only() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let (status, body) = send(&app.router, get("/readyz")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ready");
    assert_eq!(body["checks"]["eventBus"], "ok");

    // Bus outage: still ready, but reported as unavailable/degraded.
    app.bus_healthy
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let (status, body) = send(&app.router, get("/readyz")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "ready");
    assert_eq!(body["checks"]["database"], "ok");
    assert_eq!(body["checks"]["eventBus"], "unavailable");
    assert!(body["checks"]["eventBusError"]
        .as_str()
        .unwrap()
        .contains("simulated outage"));
    assert_eq!(body["degraded"], json!(["eventBus"]));
}

#[tokio::test]
async fn execution_sse_stream_ends_on_shutdown() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let (status, body) = send(
        &app.router,
        post(
            &format!("/api/v1/agents/{}/executions", app.agent_id),
            json!({ "task": "long running" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let execution_id = Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();
    app.store
        .claim_execution(execution_id, "test-worker")
        .await
        .unwrap();
    app.store
        .mark_execution_running(execution_id, Some("session-sse"))
        .await
        .unwrap();

    let uri = format!("/api/v1/executions/{execution_id}/events/stream");
    let response = app
        .router
        .clone()
        .oneshot(get(&uri))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body();

    // The execution never finishes, so the stream only ends because of the token.
    let shutdown = app.shutdown.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        shutdown.cancel();
    });
    let started = std::time::Instant::now();
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(5), body.collect())
        .await
        .expect("SSE stream must end shortly after shutdown")
        .unwrap()
        .to_bytes();
    assert!(started.elapsed() >= std::time::Duration::from_millis(1400));
    assert!(String::from_utf8_lossy(&bytes).contains("event: shutdown"));
}

#[tokio::test]
async fn unauthenticated_requests_are_rejected_when_anonymous_disabled() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let (status, body) = send(&app.router, get("/api/v1/agents")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["code"], "unauthorized");
    assert!(!body["error"]["requestId"].as_str().unwrap_or("").is_empty());

    let authed = Request::builder()
        .uri("/api/v1/agents")
        .header("authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app.router, authed).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn dev_auth_without_anonymous_requires_exact_dev_token() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let with_token = |token: &str| {
        Request::builder()
            .uri("/api/v1/agents")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    let (status, _) = send(&app.router, get("/api/v1/agents")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    for wrong in [
        "wrong-token",
        "test-toke",
        "test-token-x",
        "orbit_sk_unknown",
    ] {
        let (status, body) = send(&app.router, with_token(wrong)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "token {wrong}");
        assert_eq!(body["error"]["code"], "unauthorized");
    }
    let (status, _) = send(&app.router, with_token("test-token")).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn dev_auth_with_anonymous_accepts_missing_and_non_dev_tokens() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let (status, _) = send(&app.router, get("/api/v1/agents")).await;
    assert_eq!(status, StatusCode::OK);
    let req = Request::builder()
        .uri("/api/v1/agents")
        .header("authorization", "Bearer something-else")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app.router, req).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn agent_crud_validation_and_export() {
    let Some(app) = test_app(true).await else {
        return;
    };

    let (status, body) = send(&app.router, get("/api/v1/agents?limit=10")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["harness"], "mock");

    let (status, body) = send(&app.router, post("/api/v1/agents", definition())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["telegram_test"], Value::Null);
    assert_eq!(body["spec"]["autonomy"]["level"], 2);

    // Unknown harness → validation error envelope.
    let mut invalid = definition();
    invalid["spec"]["harness"]["type"] = json!("not-a-harness");
    let (status, body) = send(&app.router, post("/api/v1/agents", invalid)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "validation_error");

    // Semantic validation (bad cron) → 422 with field details.
    let mut invalid = definition();
    invalid["spec"]["triggers"] = json!([{ "type": "schedule", "cron": "nope" }]);
    invalid["metadata"]["name"] = json!("Invalid Agent");
    let (status, body) = send(&app.router, post("/api/v1/agents", invalid)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "validation_error");
    assert!(body["error"]["details"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["path"].as_str().unwrap_or("").contains("triggers")));

    // Export
    let (status, _) = send(&app.router, get(&format!("/api/v1/agents/{id}/export"))).await;
    assert_eq!(status, StatusCode::OK);

    // 404 envelope
    let (status, body) = send(
        &app.router,
        get(&format!("/api/v1/agents/{}", Uuid::now_v7())),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn execution_creation_is_idempotent_and_streams_events() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let path = format!("/api/v1/agents/{}/executions", app.agent_id);
    let request = |key: &str| {
        Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .header("idempotency-key", key)
            .body(Body::from(json!({ "task": "test task" }).to_string()))
            .unwrap()
    };
    let (status, body) = send(&app.router, request("api-test-idem")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let execution_id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["status"], "queued");

    let (status, body2) = send(&app.router, request("api-test-idem")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body2["id"], body["id"]);

    // Simulate the worker's durable writes so we can verify read endpoints.
    let execution_id = Uuid::parse_str(&execution_id).unwrap();
    app.store
        .claim_execution(execution_id, "test-worker")
        .await
        .unwrap();
    app.store
        .mark_execution_running(execution_id, Some("session-test"))
        .await
        .unwrap();
    app.store
        .append_execution_event(
            app.org_id,
            execution_id,
            "log",
            orbit_domain::LogLevel::Info,
            "hello from test",
            &json!({}),
        )
        .await
        .unwrap();
    app.store
        .finish_execution(
            execution_id,
            orbit_domain::ExecutionStatus::Succeeded,
            Some(&json!({ "ok": true })),
            None,
            None,
            Some("session-test"),
        )
        .await
        .unwrap();

    let (status, body) = send(
        &app.router,
        get(&format!("/api/v1/executions/{execution_id}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "succeeded");
    assert_eq!(body["output"]["ok"], true);

    let (status, body) = send(
        &app.router,
        get(&format!("/api/v1/executions/{execution_id}/events")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["message"] == "hello from test"));

    // SSE replay includes snapshot, the stored event and a done marker.
    let response = app
        .router
        .clone()
        .oneshot(get(&format!(
            "/api/v1/executions/{execution_id}/events/stream"
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // The envelope middleware must leave streams alone.
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("event: snapshot"), "{text}");
    assert!(text.contains("event: execution.event"), "{text}");
    assert!(text.contains("event: done"), "{text}");
}

#[tokio::test]
async fn approvals_can_be_listed_and_decided() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let _serial = APPROVAL_SETTINGS_LOCK.lock().await;
    reset_approval_policy(&app).await;
    let (status, execution) = send(
        &app.router,
        post(
            &format!("/api/v1/agents/{}/executions", app.agent_id),
            json!({ "task": "approval test" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{execution}");
    let execution_id = Uuid::parse_str(execution["id"].as_str().unwrap()).unwrap();
    let approval = app
        .store
        .create_approval(
            app.org_id,
            execution_id,
            json!({ "capability": "issues.comment", "effect": "write" }),
            None,
        )
        .await
        .unwrap();

    let (status, body) = send(&app.router, get("/api/v1/approvals?status=pending")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["id"] == approval.id.to_string()));

    let (status, body) = send(
        &app.router,
        post(
            &format!("/api/v1/approvals/{}/approve", approval.id),
            json!({ "comment": "looks good" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "approved");
    assert_eq!(body["comment"], "looks good");

    // Deciding twice is a conflict, not a silent overwrite.
    let (status, body) = send(
        &app.router,
        post(
            &format!("/api/v1/approvals/{}/approve", approval.id),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "conflict");
}

#[tokio::test]
async fn provider_credentials_are_write_only() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let body = json!({
        "name": "Test Gateway",
        "kind": "openai_compatible",
        "config": { "baseUrl": "http://localhost:9999/v1", "model": "qwen" },
        "credential": "sk-test-should-never-leak"
    });
    let (status, body) = send(&app.router, post("/api/v1/providers", body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["hasCredential"], true);
    assert!(!body.to_string().contains("sk-test-should-never-leak"));
    let provider_id = body["id"].as_str().unwrap();

    let (status, body) = send(&app.router, get("/api/v1/providers")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.to_string().contains("sk-test-should-never-leak"));
    assert_eq!(body[0]["hasCredential"], true);

    // Secrets are stored encrypted.
    let ciphertext: String =
        sqlx::query_scalar("SELECT ciphertext FROM secrets WHERE reference LIKE 'provider/%'")
            .fetch_one(app.store.pool())
            .await
            .unwrap();
    assert!(!ciphertext.contains("sk-test-should-never-leak"));

    // Invalid provider config is rejected before persistence.
    let (status, body) = send(
        &app.router,
        post(
            "/api/v1/providers",
            json!({ "name": "Bad", "kind": "openai_compatible", "config": { "baseUrl": "ftp://x", "model": "m" } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "validation_error");

    let (status, _) = send(
        &app.router,
        get(&format!("/api/v1/providers/{provider_id}")),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn connections_are_org_scoped_and_credentials_hidden() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let body = json!({
        "connectorSlug": "telegram",
        "name": "Telegram Bot",
        "credential": "123456:bot-secret",
        "scopes": ["messages.list"]
    });
    let (status, body) = send(&app.router, post("/api/v1/connections", body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["hasCredential"], true);
    assert_eq!(body["connectorSlug"], "telegram");
    assert!(!body.to_string().contains("bot-secret"));
    let connection_id = body["id"].as_str().unwrap().to_string();

    let (status, body) = send(&app.router, get("/api/v1/connections")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);

    // Unknown connector → 404
    let (status, body) = send(
        &app.router,
        post(
            "/api/v1/connections",
            json!({ "connectorSlug": "nope", "name": "x" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/connections/{connection_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

fn auth_get(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn auth_delete(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

async fn token_for(store: &Store, org_id: Uuid, user_id: Uuid, name: &str) -> String {
    let issued = orbit_security::issue_api_token();
    store
        .create_api_token(
            org_id,
            user_id,
            name,
            &issued.prefix,
            &issued.hash,
            &[],
            None,
        )
        .await
        .expect("api token");
    issued.token
}

#[tokio::test]
async fn private_agents_are_hidden_from_other_users() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = app
        .store
        .ensure_user(app.org_id, "alice@test.local", "Alice", UserRole::Operator)
        .await
        .unwrap();
    let bob = app
        .store
        .ensure_user(app.org_id, "bob@test.local", "Bob", UserRole::Operator)
        .await
        .unwrap();
    let alice_agent = create_demo_agent(&app.store, app.org_id, alice.id).await;
    let bob_agent = create_demo_agent(&app.store, app.org_id, bob.id).await;
    make_private(&app.store, app.org_id, "agents", alice_agent, alice.id).await;
    make_private(&app.store, app.org_id, "agents", bob_agent, bob.id).await;
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;

    let (status, body) = send(&app.router, auth_get("/api/v1/agents", &alice_token)).await;
    assert_eq!(status, StatusCode::OK);
    let items = body["items"].as_array().unwrap();
    assert!(items.iter().all(|item| item["id"] != bob_agent.to_string()));
    assert!(items
        .iter()
        .any(|item| item["id"] == alice_agent.to_string()));

    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/agents/{bob_agent}"), &alice_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/agents/{alice_agent}"), &bob_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/agents/{alice_agent}"), &alice_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(
        &app.router,
        auth_delete(&format!("/api/v1/agents/{alice_agent}"), &bob_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app.router,
        auth_patch(
            &format!("/api/v1/agents/{alice_agent}"),
            &bob_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/agents/{alice_agent}/export"), &bob_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agents/{alice_agent}/executions"),
            &bob_token,
            json!({ "task": "cross-team execution" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Sharing to org visibility makes the same resource visible again.
    app.store
        .set_resource_visibility(app.org_id, "agents", bob_agent, "org", None)
        .await
        .unwrap();
    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/agents/{bob_agent}"), &alice_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Back to private: hidden again.
    app.store
        .set_resource_visibility(app.org_id, "agents", bob_agent, "private", None)
        .await
        .unwrap();
    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/agents/{bob_agent}"), &alice_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

fn auth_patch(path: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("PATCH")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn auth_post(path: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn private_skills_are_hidden_from_other_users() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = app
        .store
        .ensure_user(
            app.org_id,
            "skills-alice@test.local",
            "Alice",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let bob = app
        .store
        .ensure_user(
            app.org_id,
            "skills-bob@test.local",
            "Bob",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;

    let (status, alice_skill) = send(
        &app.router,
        auth_post(
            "/api/v1/skills",
            &alice_token,
            json!({
                "name": "Alice skill",
                "slug": "alice-skill",
                "content": "alice",
                "files": {}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, bob_skill) = send(
        &app.router,
        auth_post(
            "/api/v1/skills",
            &bob_token,
            json!({
                "name": "Bob skill",
                "slug": "bob-skill",
                "content": "bob",
                "files": {}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let alice_id = alice_skill["id"].as_str().unwrap();
    let bob_id = bob_skill["id"].as_str().unwrap();
    for id in [alice_id, bob_id] {
        app.store
            .set_resource_visibility(
                app.org_id,
                "skills",
                Uuid::parse_str(id).unwrap(),
                "private",
                None,
            )
            .await
            .unwrap();
    }

    let (status, body) = send(&app.router, auth_get("/api/v1/skills", &alice_token)).await;
    assert_eq!(status, StatusCode::OK);
    let items = body.as_array().unwrap();
    assert!(items.iter().any(|item| item["id"] == alice_id));
    assert!(items.iter().all(|item| item["id"] != bob_id));

    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/skills/{bob_id}"), &alice_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app.router,
        auth_delete(&format!("/api/v1/skills/{bob_id}"), &alice_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn make_private(store: &Store, org_id: Uuid, table: &str, id: Uuid, user_id: Uuid) {
    store
        .assign_resource_owner(org_id, table, id, user_id)
        .await
        .unwrap();
    store
        .set_resource_visibility(org_id, table, id, "private", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn private_resources_are_hidden_from_other_users() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = app
        .store
        .ensure_user(
            app.org_id,
            "res-alice@test.local",
            "Alice",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let bob = app
        .store
        .ensure_user(app.org_id, "res-bob@test.local", "Bob", UserRole::Operator)
        .await
        .unwrap();
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;
    let alice_agent = create_demo_agent(&app.store, app.org_id, alice.id).await;
    let bob_agent = create_demo_agent(&app.store, app.org_id, bob.id).await;
    let connector = app
        .store
        .get_connector_by_slug(app.org_id, "webhook")
        .await
        .unwrap()
        .expect("webhook connector");

    let alice_provider = app
        .store
        .create_provider(
            app.org_id,
            "Alice provider",
            "openai_compatible",
            &json!({ "baseUrl": "http://127.0.0.1:9/v1" }),
            None,
        )
        .await
        .unwrap();
    let bob_provider = app
        .store
        .create_provider(
            app.org_id,
            "Bob provider",
            "openai_compatible",
            &json!({ "baseUrl": "http://127.0.0.1:9/v1" }),
            None,
        )
        .await
        .unwrap();
    let alice_connection = app
        .store
        .create_connection(
            app.org_id,
            connector.id,
            "Alice webhook",
            &json!({ "url": "http://127.0.0.1:9/alice" }),
            &[],
            None,
            Some(alice.id),
        )
        .await
        .unwrap();
    let bob_connection = app
        .store
        .create_connection(
            app.org_id,
            connector.id,
            "Bob webhook",
            &json!({ "url": "http://127.0.0.1:9/bob" }),
            &[],
            None,
            Some(bob.id),
        )
        .await
        .unwrap();
    let alice_worker = app
        .store
        .create_worker_profile(
            app.org_id,
            "Alice worker",
            "mock",
            "local",
            "process",
            "orbit-worker",
            "test",
            "all",
            &[],
            &[],
            None,
            None,
            None,
            &json!({}),
            &json!({}),
            &json!({}),
            false,
            false,
            false,
            false,
            &json!({}),
        )
        .await
        .unwrap();
    let bob_worker = app
        .store
        .create_worker_profile(
            app.org_id,
            "Bob worker",
            "mock",
            "local",
            "process",
            "orbit-worker",
            "test",
            "all",
            &[],
            &[],
            None,
            None,
            None,
            &json!({}),
            &json!({}),
            &json!({}),
            false,
            false,
            false,
            false,
            &json!({}),
        )
        .await
        .unwrap();
    let alice_trigger = app
        .store
        .create_trigger(app.org_id, alice_agent, "manual", None, json!({}), true)
        .await
        .unwrap();
    let bob_trigger = app
        .store
        .create_trigger(app.org_id, bob_agent, "manual", None, json!({}), true)
        .await
        .unwrap();
    let alice_policy = app
        .store
        .create_tool_policy(
            app.org_id,
            None,
            "Alice policy",
            100,
            true,
            "allow",
            &json!({}),
            None,
            None,
        )
        .await
        .unwrap();
    let bob_policy = app
        .store
        .create_tool_policy(
            app.org_id,
            None,
            "Bob policy",
            100,
            true,
            "allow",
            &json!({}),
            None,
            None,
        )
        .await
        .unwrap();

    for (table, alice_id, bob_id) in [
        ("providers", alice_provider.id, bob_provider.id),
        ("connections", alice_connection.id, bob_connection.id),
        ("worker_profiles", alice_worker.id, bob_worker.id),
        ("triggers", alice_trigger.id, bob_trigger.id),
        ("tool_policies", alice_policy.id, bob_policy.id),
    ] {
        make_private(&app.store, app.org_id, table, alice_id, alice.id).await;
        make_private(&app.store, app.org_id, table, bob_id, bob.id).await;
    }

    for (path, bob_id, has_get) in [
        ("/api/v1/providers", bob_provider.id, false),
        ("/api/v1/connections", bob_connection.id, false),
        ("/api/v1/worker-profiles", bob_worker.id, true),
        ("/api/v1/triggers", bob_trigger.id, false),
        ("/api/v1/tool-policies", bob_policy.id, false),
    ] {
        let (status, body) = send(&app.router, auth_get(path, &alice_token)).await;
        assert_eq!(status, StatusCode::OK, "{path} list");
        let items = body
            .as_array()
            .unwrap_or_else(|| panic!("{path} list is not an array"));
        assert!(
            items.iter().all(|item| item["id"] != bob_id.to_string()),
            "{path} leaked another user's resource"
        );

        if has_get {
            let (status, _) = send(
                &app.router,
                auth_get(&format!("{path}/{bob_id}"), &alice_token),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}/{{id}} get");
        }

        let (status, _) = send(
            &app.router,
            auth_delete(&format!("{path}/{bob_id}"), &alice_token),
        )
        .await;
        // Operators lack write permission for some resources; both 403 (RBAC) and
        // 404 (hidden by visibility) are acceptable and must never be 2xx.
        assert!(
            matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
            "{path}/{{id}} delete returned {status}"
        );
    }

    // The owner still sees their own resource.
    let (status, body) = send(&app.router, auth_get("/api/v1/providers", &bob_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["id"] == bob_provider.id.to_string()));
}

#[tokio::test]
async fn private_agent_teams_and_runs_are_hidden_from_other_users() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = app
        .store
        .ensure_user(
            app.org_id,
            "team-alice@test.local",
            "Alice",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let bob = app
        .store
        .ensure_user(app.org_id, "team-bob@test.local", "Bob", UserRole::Operator)
        .await
        .unwrap();
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;
    let alice_team = app
        .store
        .create_agent_team(
            app.org_id,
            "Alice team",
            "",
            &json!({ "steps": [] }),
            Some(alice.id),
        )
        .await
        .unwrap();
    let bob_team = app
        .store
        .create_agent_team(
            app.org_id,
            "Bob team",
            "",
            &json!({ "steps": [] }),
            Some(bob.id),
        )
        .await
        .unwrap();
    make_private(
        &app.store,
        app.org_id,
        "agent_teams",
        alice_team.id,
        alice.id,
    )
    .await;
    make_private(&app.store, app.org_id, "agent_teams", bob_team.id, bob.id).await;
    let bob_run = app
        .store
        .create_team_run(app.org_id, bob_team.id, &json!({}), Some(bob.id))
        .await
        .unwrap();

    let (status, body) = send(&app.router, auth_get("/api/v1/agent-teams", &alice_token)).await;
    assert_eq!(status, StatusCode::OK);
    let items = body.as_array().unwrap();
    assert!(items
        .iter()
        .any(|item| item["id"] == alice_team.id.to_string()));
    assert!(items
        .iter()
        .all(|item| item["id"] != bob_team.id.to_string()));

    let (status, _) = send(
        &app.router,
        auth_get(
            &format!("/api/v1/agent-teams/{}", bob_team.id),
            &alice_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = send(&app.router, auth_get("/api/v1/team-runs", &alice_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["id"] != bob_run.id.to_string()));

    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/team-runs/{}", bob_run.id), &alice_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agent-teams/{}/runs", bob_team.id),
            &alice_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Bob still sees his own run.
    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/team-runs/{}", bob_run.id), &bob_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn private_execution_events_and_approvals_are_hidden() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = app
        .store
        .ensure_user(
            app.org_id,
            "exec-alice@test.local",
            "Alice",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let bob = app
        .store
        .ensure_user(app.org_id, "exec-bob@test.local", "Bob", UserRole::Operator)
        .await
        .unwrap();
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;
    let bob_agent = create_demo_agent(&app.store, app.org_id, bob.id).await;
    make_private(&app.store, app.org_id, "agents", bob_agent, bob.id).await;

    let (status, execution) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agents/{bob_agent}/executions"),
            &bob_token,
            json!({ "task": "private execution" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let execution_id = execution["id"].as_str().unwrap();

    for path in [
        format!("/api/v1/executions/{execution_id}"),
        format!("/api/v1/executions/{execution_id}/events"),
        format!("/api/v1/executions/{execution_id}/events/stream"),
    ] {
        let (status, _) = send(&app.router, auth_get(&path, &alice_token)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
    }

    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/executions/{execution_id}/cancel"),
            &alice_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let approval = app
        .store
        .create_approval(
            app.org_id,
            Uuid::parse_str(execution_id).unwrap(),
            json!({ "action": "deploy" }),
            None,
        )
        .await
        .unwrap();

    let (status, body) = send(&app.router, auth_get("/api/v1/approvals", &alice_token)).await;
    assert!(
        status == StatusCode::FORBIDDEN
            || (status == StatusCode::OK
                && body
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["id"] != approval.id.to_string()))
    );
}

#[tokio::test]
async fn community_license_blocks_team_and_sharing_writes() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let token = "test-token";
    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/teams",
            token,
            json!({ "name": "Ops", "slug": "ops" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"]["code"], "license_required");

    let (status, body) = send(
        &app.router,
        auth_patch(
            &format!("/api/v1/sharing/agents/{}", app.agent_id),
            token,
            json!({ "visibility": "private" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(body["error"]["code"], "license_required");
}

#[tokio::test]
async fn same_agent_queues_on_different_harnesses() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let mut value = definition();
    value["spec"].as_object_mut().unwrap().remove("harness");
    value["spec"]["resources"]["maxConcurrentExecutions"] = json!(2);
    let definition: orbit_domain::AgentDefinition = serde_json::from_value(value).unwrap();
    let owner = app
        .store
        .list_users(app.org_id)
        .await
        .unwrap()
        .into_iter()
        .find(|user| user.role == "owner")
        .unwrap();
    let (agent, _) = app
        .store
        .create_agent(app.org_id, Some(owner.id), &definition)
        .await
        .unwrap();
    app.store
        .set_agent_status(app.org_id, agent.id, orbit_domain::AgentStatus::Active)
        .await
        .unwrap();
    app.store
        .set_platform_setting(
            "harness_default",
            &json!({ "default": "mock" }),
            Some(owner.id),
        )
        .await
        .unwrap();

    let (status, first) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agents/{}/executions", agent.id),
            "test-token",
            json!({ "task": "default harness" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(first["harnessKind"], "mock");

    let (status, second) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agents/{}/executions", agent.id),
            "test-token",
            json!({ "task": "pi override", "harness": "pi" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(second["harnessKind"], "pi");
}

/// `runtime_tick` advances every runtime in the shared test database, so tests that tick
/// (or interleave with a tick) run one at a time.
static RUNTIME_TICK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Approval policy is a platform-wide setting; tests that change it must not overlap.
static APPROVAL_SETTINGS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn reset_approval_policy(app: &TestApp) {
    app.store
        .set_platform_setting(
            "approvals",
            &json!({ "approvalsRequireDifferentApprover": false }),
            None,
        )
        .await
        .unwrap();
}

async fn raw_pool() -> sqlx::PgPool {
    sqlx::PgPool::connect(&std::env::var("ORBIT_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn poisoned_runtimes_do_not_block_healthy_ones() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let _serial = RUNTIME_TICK_LOCK.lock().await;
    let user = app
        .store
        .ensure_user(app.org_id, "api@test.local", "API Test", UserRole::Owner)
        .await
        .unwrap();
    let pool = raw_pool().await;

    // Oldest runtime: unknown harness (permanent error).
    let bad_harness = create_demo_agent(&app.store, app.org_id, user.id).await;
    let rt_harness = app
        .store
        .create_agent_runtime(app.org_id, bad_harness, "t", "bogus", 60, None)
        .await
        .unwrap();
    // Second: current spec that no longer deserialises (transient, backs off).
    let bad_spec = create_demo_agent(&app.store, app.org_id, user.id).await;
    sqlx::query("UPDATE agent_versions SET spec = '{\"bogus\": true}'::jsonb WHERE agent_id = $1")
        .bind(bad_spec)
        .execute(&pool)
        .await
        .unwrap();
    app.store
        .create_agent_runtime(app.org_id, bad_spec, "t", "mock", 10, None)
        .await
        .unwrap();
    // Healthy runtime created last.
    let healthy = create_demo_agent(&app.store, app.org_id, user.id).await;
    app.store
        .create_agent_runtime(app.org_id, healthy, "t", "mock", 60, None)
        .await
        .unwrap();

    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();

    let ok = app
        .store
        .get_agent_runtime(app.org_id, healthy)
        .await
        .unwrap()
        .unwrap();
    assert!(
        ok.last_execution_id.is_some(),
        "healthy runtime must advance"
    );
    assert!(ok.last_error.is_none());

    let spec = app
        .store
        .get_agent_runtime(app.org_id, bad_spec)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(spec.status, "running");
    assert!(spec.last_error.as_deref().unwrap().contains("invalid"));
    assert!(spec.next_run_at > chrono::Utc::now() + chrono::Duration::seconds(30));
    assert!(spec.last_execution_id.is_none());

    let (status, error): (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error FROM agent_runtimes WHERE id = $1")
            .bind(rt_harness.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "stopped");
    assert!(error.unwrap().contains("unknown harness"));
}

#[tokio::test]
async fn runtime_tick_retry_reuses_execution() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let _serial = RUNTIME_TICK_LOCK.lock().await;
    let user = app
        .store
        .ensure_user(app.org_id, "api@test.local", "API Test", UserRole::Owner)
        .await
        .unwrap();
    let agent_id = create_demo_agent(&app.store, app.org_id, user.id).await;
    let runtime = app
        .store
        .create_agent_runtime(app.org_id, agent_id, "t", "mock", 60, None)
        .await
        .unwrap();
    let pool = raw_pool().await;

    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();
    let first = app
        .store
        .get_agent_runtime(app.org_id, agent_id)
        .await
        .unwrap()
        .unwrap()
        .last_execution_id
        .unwrap();

    // Simulate a crash between create_execution and update_agent_runtime: the execution
    // exists but the runtime row still points at nothing and is due again.
    sqlx::query("UPDATE agent_runtimes SET last_execution_id = NULL, next_run_at = now() - interval '1 second' WHERE id = $1")
        .bind(runtime.id)
        .execute(&pool)
        .await
        .unwrap();
    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();

    let retried = app
        .store
        .get_agent_runtime(app.org_id, agent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retried.last_execution_id,
        Some(first),
        "retry reuses the execution"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM executions WHERE org_id = $1 AND agent_id = $2")
            .bind(app.org_id)
            .bind(agent_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1, "no duplicate execution");
}

async fn runtime_row(
    app: &TestApp,
    agent_id: uuid::Uuid,
) -> orbit_store::agent_runtimes::AgentRuntime {
    sqlx::query_as("SELECT * FROM agent_runtimes WHERE org_id = $1 AND agent_id = $2")
        .bind(app.org_id)
        .bind(agent_id)
        .fetch_one(&raw_pool().await)
        .await
        .unwrap()
}

async fn fail_execution(pool: &sqlx::PgPool, execution_id: uuid::Uuid) {
    sqlx::query("UPDATE executions SET status = 'failed', error = '\"boom\"'::jsonb, finished_at = now() WHERE id = $1")
        .bind(execution_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn make_runtime_due(pool: &sqlx::PgPool, runtime_id: uuid::Uuid) {
    sqlx::query(
        "UPDATE agent_runtimes SET next_run_at = now() - interval '1 second' WHERE id = $1",
    )
    .bind(runtime_id)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn runtime_stops_after_consecutive_failures() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let _serial = RUNTIME_TICK_LOCK.lock().await;
    let user = app
        .store
        .ensure_user(app.org_id, "api@test.local", "API Test", UserRole::Owner)
        .await
        .unwrap();
    let agent_id = create_demo_agent(&app.store, app.org_id, user.id).await;
    let runtime = app
        .store
        .create_agent_runtime(app.org_id, agent_id, "t", "mock", 60, None)
        .await
        .unwrap();
    let pool = raw_pool().await;
    let limit = orbit_api::background::RUNTIME_MAX_CONSECUTIVE_FAILURES;

    for round in 1..=limit {
        make_runtime_due(&pool, runtime.id).await;
        orbit_api::background::runtime_tick(&app.state)
            .await
            .unwrap();
        let id = runtime_row(&app, agent_id).await.last_execution_id.unwrap();
        fail_execution(&pool, id).await;
        make_runtime_due(&pool, runtime.id).await;
        orbit_api::background::runtime_tick(&app.state)
            .await
            .unwrap();
        let row = runtime_row(&app, agent_id).await;
        assert_eq!(row.consecutive_failures, round);
        if round < limit {
            assert_eq!(row.status, "running");
        }
    }
    let row = runtime_row(&app, agent_id).await;
    assert_eq!(row.status, "stopped");
    assert!(row.last_error.as_deref().unwrap().contains(&format!(
        "stopped after {limit} consecutive failed executions"
    )));
}

async fn new_execution(app: &TestApp, agent_id: Uuid) -> Uuid {
    let (status, execution) = send(
        &app.router,
        post(
            &format!("/api/v1/agents/{agent_id}/executions"),
            json!({ "task": "approval policy test" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{execution}");
    Uuid::parse_str(execution["id"].as_str().unwrap()).unwrap()
}

async fn new_approval(app: &TestApp, execution_id: Uuid) -> Uuid {
    app.store
        .create_approval(
            app.org_id,
            execution_id,
            json!({ "action": "deploy" }),
            None,
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn approval_decisions_require_use_and_optional_four_eyes() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let _serial = APPROVAL_SETTINGS_LOCK.lock().await;
    reset_approval_policy(&app).await;
    app.store
        .create_custom_role(
            app.org_id,
            "Approver",
            &[
                "agents:read".into(),
                "executions:read".into(),
                "approvals:decide".into(),
            ],
        )
        .await
        .unwrap();
    let carol = app
        .store
        .ensure_user(
            app.org_id,
            "appr-carol@test.local",
            "Carol",
            UserRole::Viewer,
        )
        .await
        .unwrap();
    app.store
        .set_user_role_name(app.org_id, carol.id, "Approver")
        .await
        .unwrap();
    let carol_token = token_for(&app.store, app.org_id, carol.id, "carol").await;
    let team = app
        .store
        .create_access_team(
            app.org_id,
            "Approvers",
            &format!("appr-{}", Uuid::now_v7()),
            None,
        )
        .await
        .unwrap();
    // Someone other than Carol owns the agent, so only her team role grants access.
    let dave = app
        .store
        .ensure_user(
            app.org_id,
            "appr-dave@test.local",
            "Dave",
            UserRole::Operator,
        )
        .await
        .unwrap();
    app.store
        .assign_resource_owner(app.org_id, "agents", app.agent_id, dave.id)
        .await
        .unwrap();
    app.store
        .set_resource_visibility(app.org_id, "agents", app.agent_id, "team", Some(team.id))
        .await
        .unwrap();
    app.store
        .upsert_team_member(team.id, carol.id, "viewer")
        .await
        .unwrap();

    let execution_id = new_execution(&app, app.agent_id).await;
    let approval_id = new_approval(&app, execution_id).await;

    // Team viewer: can see the agent but cannot use it -> 403.
    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/approvals/{approval_id}/approve"),
            &carol_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Not in the team at all -> the agent is invisible -> 404.
    app.store
        .remove_team_member(team.id, carol.id)
        .await
        .unwrap();
    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/approvals/{approval_id}/approve"),
            &carol_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Team member: allowed (four-eyes off by default; requester is the dev owner).
    app.store
        .upsert_team_member(team.id, carol.id, "member")
        .await
        .unwrap();
    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/approvals/{approval_id}/approve"),
            &carol_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Default setting is off.
    let (status, body) = send(&app.router, get("/api/v1/settings/approvals")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["approvalsRequireDifferentApprover"], false);

    // Four-eyes off: the requester (dev owner) may approve their own execution.
    let own = new_approval(&app, execution_id).await;
    let (status, body) = send(
        &app.router,
        post(&format!("/api/v1/approvals/{own}/approve"), json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Four-eyes on: the requester is refused, another user is not.
    let request = Request::builder()
        .method("PUT")
        .uri("/api/v1/settings/approvals")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "approvalsRequireDifferentApprover": true }).to_string(),
        ))
        .unwrap();
    let (status, body) = send(&app.router, request).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["approvalsRequireDifferentApprover"], true);

    let gated = new_approval(&app, execution_id).await;
    for decision in ["approve", "reject"] {
        let (status, body) = send(
            &app.router,
            post(&format!("/api/v1/approvals/{gated}/{decision}"), json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("different approver"));
    }
    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/approvals/{gated}/approve"),
            &carol_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "approved");
}

#[tokio::test]
async fn expired_approval_cannot_be_decided_via_api() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let _serial = APPROVAL_SETTINGS_LOCK.lock().await;
    reset_approval_policy(&app).await;
    let execution_id = new_execution(&app, app.agent_id).await;
    let approval = app
        .store
        .create_approval(
            app.org_id,
            execution_id,
            json!({ "action": "late" }),
            Some(chrono::Utc::now() - chrono::Duration::seconds(30)),
        )
        .await
        .unwrap();
    let (status, body) = send(
        &app.router,
        post(
            &format!("/api/v1/approvals/{}/approve", approval.id),
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("approval expired"));
    assert_eq!(
        app.store
            .get_approval(app.org_id, approval.id)
            .await
            .unwrap()
            .status,
        "expired"
    );
}

fn worker_request(method: &str, path: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

#[tokio::test]
async fn worker_approvals_are_bound_to_the_execution_and_clamped() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let issued = orbit_security::issue_worker_token();
    let token_record = app
        .store
        .create_worker_token(
            app.org_id,
            None,
            "approvals-test",
            &issued.prefix,
            &issued.hash,
            &[],
        )
        .await
        .unwrap();
    let execution_a = new_execution(&app, app.agent_id).await;
    let zed = app
        .store
        .ensure_user(
            app.org_id,
            "worker-zed@test.local",
            "Zed",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let other_agent = create_demo_agent(&app.store, app.org_id, zed.id).await;
    let execution_b = new_execution(&app, other_agent).await;
    for execution_id in [execution_a, execution_b] {
        app.store
            .claim_execution(execution_id, "approvals-test-worker")
            .await
            .unwrap()
            .expect("execution is queued");
        app.store
            .bind_execution_worker_token(execution_id, token_record.id)
            .await
            .unwrap();
    }

    // u64::MAX must neither panic nor produce an absurd deadline.
    let (status, body) = send(
        &app.router,
        worker_request(
            "POST",
            "/api/v1/worker/approvals",
            &issued.token,
            Some(json!({
                "executionId": execution_a,
                "action": { "capability": "x" },
                "waitSeconds": u64::MAX
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let expires: chrono::DateTime<chrono::Utc> =
        body["expiresAt"].as_str().unwrap().parse().unwrap();
    assert!(
        expires <= chrono::Utc::now() + chrono::Duration::hours(24) + chrono::Duration::minutes(1)
    );
    let approval_id = Uuid::parse_str(body["id"].as_str().unwrap()).unwrap();

    // Decide it so the wait returns immediately for the matching execution.
    app.store
        .decide_approval(app.org_id, approval_id, "approved", zed.id, None)
        .await
        .unwrap();
    let (status, body) = send(
        &app.router,
        worker_request(
            "GET",
            &format!(
                "/api/v1/worker/approvals/{approval_id}/wait?executionId={execution_a}&seconds=1"
            ),
            &issued.token,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Another execution in the same org cannot read this approval's status.
    let (status, _) = send(
        &app.router,
        worker_request(
            "GET",
            &format!(
                "/api/v1/worker/approvals/{approval_id}/wait?executionId={execution_b}&seconds=1"
            ),
            &issued.token,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Per-execution cap on pending approvals.
    for _ in 0..49 {
        app.store
            .create_approval(app.org_id, execution_b, json!({ "n": 1 }), None)
            .await
            .unwrap();
    }
    let (status, _) = send(
        &app.router,
        worker_request(
            "POST",
            "/api/v1/worker/approvals",
            &issued.token,
            Some(json!({ "executionId": execution_b, "action": {} })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        &app.router,
        worker_request(
            "POST",
            "/api/v1/worker/approvals",
            &issued.token,
            Some(json!({ "executionId": execution_b, "action": {} })),
        ),
    )
    .await;
    // 49 + 1 created above = 50 pending -> the next request is refused.
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

#[tokio::test]
async fn env_mapping_secret_refs_must_be_usable_in_the_callers_org() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let mapping = |reference: &str| {
        json!({
            "name": format!("map-{}", Uuid::now_v7()),
            "mappings": [{ "envVar": "TOKEN", "secretRef": reference }]
        })
    };

    // A real secret that belongs to another organization.
    let other = app
        .store
        .create_organization(
            &format!("other-{}", Uuid::now_v7()),
            "Other",
            orbit_domain::OrganizationMode::Personal,
        )
        .await
        .unwrap();
    let foreign = format!("connection/{}", Uuid::now_v7());
    app.store
        .put_secret(
            Some(other.id),
            &foreign,
            &orbit_security::SecretValue::new("foreign-super-secret"),
            "t",
        )
        .await
        .unwrap();
    let (status, body) = send(&app.router, post("/api/v1/env-mappings", mapping(&foreign))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(!body.to_string().contains("foreign-super-secret"));

    // Unknown reference.
    let (status, _) = send(
        &app.router,
        post("/api/v1/env-mappings", mapping("connection/does-not-exist")),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Own-org secret that no connection/provider owns.
    let orphan = format!("connection/{}", Uuid::now_v7());
    app.store
        .put_secret(
            Some(app.org_id),
            &orphan,
            &orbit_security::SecretValue::new("orphan"),
            "t",
        )
        .await
        .unwrap();
    let (status, _) = send(&app.router, post("/api/v1/env-mappings", mapping(&orphan))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Unknown connection id.
    let (status, _) = send(
        &app.router,
        post(
            "/api/v1/env-mappings",
            json!({
                "name": "bad-conn",
                "mappings": [{ "envVar": "TOKEN", "connectionId": Uuid::now_v7() }]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // A usable connection's credential in the caller's org is accepted (create + update).
    let (status, created) = send(
        &app.router,
        post(
            "/api/v1/connections",
            json!({
                "connectorSlug": "telegram",
                "name": "Mapped bot",
                "credential": "123456:bot-secret",
                "scopes": ["messages.list"]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let connection = app
        .store
        .get_connection(
            app.org_id,
            Uuid::parse_str(created["id"].as_str().unwrap()).unwrap(),
        )
        .await
        .unwrap();
    let reference = connection.credential_ref.unwrap();
    let (status, body) = send(
        &app.router,
        post("/api/v1/env-mappings", mapping(&reference)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap();
    let patch = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/env-mappings/{id}"))
        .header("content-type", "application/json")
        .body(Body::from(mapping(&foreign).to_string()))
        .unwrap();
    let (status, _) = send(&app.router, patch).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn env_mapping_secret_refs_respect_resource_access() {
    let Some(app) = test_app(true).await else {
        return;
    };
    app.store
        .create_custom_role(
            app.org_id,
            "Mapper",
            &["settings:read".into(), "settings:write".into()],
        )
        .await
        .unwrap();
    let erin = app
        .store
        .ensure_user(app.org_id, "map-erin@test.local", "Erin", UserRole::Viewer)
        .await
        .unwrap();
    app.store
        .set_user_role_name(app.org_id, erin.id, "Mapper")
        .await
        .unwrap();
    let erin_token = token_for(&app.store, app.org_id, erin.id, "erin").await;
    let frank = app
        .store
        .ensure_user(
            app.org_id,
            "map-frank@test.local",
            "Frank",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let connector = app
        .store
        .get_connector_by_slug(app.org_id, "webhook")
        .await
        .unwrap()
        .expect("webhook connector");
    let reference = format!("connection/{}", Uuid::now_v7());
    app.store
        .put_secret(
            Some(app.org_id),
            &reference,
            &orbit_security::SecretValue::new("franks-token"),
            "t",
        )
        .await
        .unwrap();
    let connection = app
        .store
        .create_connection(
            app.org_id,
            connector.id,
            "Frank private",
            &json!({ "url": "http://127.0.0.1:9/frank" }),
            &[],
            Some(&reference),
            Some(frank.id),
        )
        .await
        .unwrap();
    make_private(
        &app.store,
        app.org_id,
        "connections",
        connection.id,
        frank.id,
    )
    .await;

    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/env-mappings",
            &erin_token,
            json!({
                "name": "erin-map",
                "mappings": [{ "envVar": "TOKEN", "secretRef": reference }]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(!body.to_string().contains("franks-token"));

    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/env-mappings",
            &erin_token,
            json!({
                "name": "erin-map-conn",
                "mappings": [{ "envVar": "TOKEN", "connectionId": connection.id }]
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
}

async fn runtime_with_first_execution(
    app: &TestApp,
) -> (
    orbit_store::agent_runtimes::AgentRuntime,
    uuid::Uuid,
    uuid::Uuid,
) {
    let user = app
        .store
        .ensure_user(app.org_id, "api@test.local", "API Test", UserRole::Owner)
        .await
        .unwrap();
    let agent_id = create_demo_agent(&app.store, app.org_id, user.id).await;
    let runtime = app
        .store
        .create_agent_runtime(app.org_id, agent_id, "t", "mock", 60, None)
        .await
        .unwrap();
    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();
    let first = runtime_row(app, agent_id).await.last_execution_id.unwrap();
    (runtime, agent_id, first)
}

async fn execution_count(pool: &sqlx::PgPool, agent_id: uuid::Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM executions WHERE agent_id = $1")
        .bind(agent_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn stop_during_inflight_tick_is_not_overwritten_by_launch() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let _serial = RUNTIME_TICK_LOCK.lock().await;
    let pool = raw_pool().await;
    let (runtime, agent_id, exec) = runtime_with_first_execution(&app).await;

    // Rewind to "tick claimed, execution created, runtime row not yet updated".
    sqlx::query("UPDATE agent_runtimes SET last_execution_id = NULL, next_run_at = now() - interval '1 second' WHERE id = $1")
        .bind(runtime.id)
        .execute(&pool)
        .await
        .unwrap();
    let claimed = app
        .store
        .claim_agent_runtime_tick(
            runtime.id,
            chrono::Utc::now() + chrono::Duration::seconds(5),
        )
        .await
        .unwrap()
        .expect("claimed");

    // Operator presses Stop while the tick is in flight.
    let stopping = app
        .store
        .request_agent_runtime_stop(runtime.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stopping.status, "stopping");

    // The tick's final write must not revive the runtime, but must record the execution.
    let recorded = app
        .store
        .record_agent_runtime_launch(
            runtime.id,
            exec,
            true,
            chrono::Utc::now() + chrono::Duration::seconds(60),
        )
        .await
        .unwrap()
        .expect("launch is recorded for a stopping runtime");
    assert_eq!(recorded.status, "stopping");
    assert_eq!(recorded.last_execution_id, Some(exec));
    assert_eq!(
        recorded.next_run_at, claimed.next_run_at,
        "schedule untouched"
    );

    // The stopping branch cancels the raced execution, then finishes the stop.
    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();
    assert_eq!(runtime_row(&app, agent_id).await.status, "stopping");
    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();
    let row = runtime_row(&app, agent_id).await;
    assert_eq!(row.status, "stopped");
    assert!(row.stopped_at.is_some());
    let status: String = sqlx::query_scalar("SELECT status FROM executions WHERE id = $1")
        .bind(exec)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");

    // No further executions, ever.
    make_runtime_due(&pool, runtime.id).await;
    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();
    assert_eq!(runtime_row(&app, agent_id).await.status, "stopped");
    assert_eq!(execution_count(&pool, agent_id).await, 1);
}

#[tokio::test]
async fn stop_during_inflight_tick_is_not_overwritten_by_backoff() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let _serial = RUNTIME_TICK_LOCK.lock().await;
    let pool = raw_pool().await;
    let (runtime, agent_id, first) = runtime_with_first_execution(&app).await;
    fail_execution(&pool, first).await;
    make_runtime_due(&pool, runtime.id).await;
    let claimed = app
        .store
        .claim_agent_runtime_tick(
            runtime.id,
            chrono::Utc::now() + chrono::Duration::seconds(5),
        )
        .await
        .unwrap()
        .expect("claimed");

    app.store
        .request_agent_runtime_stop(runtime.id)
        .await
        .unwrap()
        .unwrap();

    // Failure accounting, plain backoff and the stop-at-limit write are all refused.
    let later = chrono::Utc::now() + chrono::Duration::seconds(300);
    assert!(app
        .store
        .record_agent_runtime_failure(runtime.id, false, first, 1, "boom", later)
        .await
        .unwrap()
        .is_none());
    assert!(app
        .store
        .record_agent_runtime_failure(runtime.id, true, first, 5, "boom", later)
        .await
        .unwrap()
        .is_none());
    assert!(app
        .store
        .advance_agent_runtime(runtime.id, Some("transient"), later)
        .await
        .unwrap()
        .is_none());
    assert!(app
        .store
        .halt_running_agent_runtime(runtime.id, "permanent")
        .await
        .unwrap()
        .is_none());

    let row = runtime_row(&app, agent_id).await;
    assert_eq!(row.status, "stopping");
    assert_eq!(row.consecutive_failures, 0);
    assert_eq!(row.next_run_at, claimed.next_run_at);

    // The last execution is terminal, so the next tick completes the stop.
    orbit_api::background::runtime_tick(&app.state)
        .await
        .unwrap();
    assert_eq!(runtime_row(&app, agent_id).await.status, "stopped");
    assert_eq!(execution_count(&pool, agent_id).await, 1);
}

#[tokio::test]
async fn runtime_status_transitions_are_guarded() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let _serial = RUNTIME_TICK_LOCK.lock().await;
    let (runtime, agent_id, exec) = runtime_with_first_execution(&app).await;
    let soon = chrono::Utc::now() + chrono::Duration::seconds(60);

    // stopping -> stopped only from stopping.
    assert!(app
        .store
        .finish_agent_runtime_stop(runtime.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(runtime_row(&app, agent_id).await.status, "running");

    app.store
        .request_agent_runtime_stop(runtime.id)
        .await
        .unwrap()
        .unwrap();
    // Stop is idempotent while stopping.
    assert_eq!(
        app.store
            .request_agent_runtime_stop(runtime.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "stopping"
    );
    app.store
        .finish_agent_runtime_stop(runtime.id)
        .await
        .unwrap()
        .unwrap();

    // Once stopped nothing moves it: no restart, no schedule, no launch, no second stop.
    assert!(app
        .store
        .record_agent_runtime_launch(runtime.id, exec, true, soon)
        .await
        .unwrap()
        .is_none());
    assert!(app
        .store
        .advance_agent_runtime(runtime.id, None, soon)
        .await
        .unwrap()
        .is_none());
    assert!(app
        .store
        .halt_agent_runtime(runtime.id, "x")
        .await
        .unwrap()
        .is_none());
    assert!(app
        .store
        .request_agent_runtime_stop(runtime.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(runtime_row(&app, agent_id).await.status, "stopped");
}

#[tokio::test]
async fn disabled_users_tokens_are_rejected() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let user = app
        .store
        .ensure_user(app.org_id, "off@test.local", "Off", UserRole::Owner)
        .await
        .unwrap();
    let token = token_for(&app.store, app.org_id, user.id, "off").await;
    let (status, _) = send(&app.router, auth_get("/api/v1/me", &token)).await;
    assert_eq!(status, StatusCode::OK);
    sqlx::query("UPDATE users SET status = 'disabled' WHERE id = $1")
        .bind(user.id)
        .execute(app.store.pool())
        .await
        .unwrap();
    let (status, _) = send(&app.router, auth_get("/api/v1/me", &token)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

fn put_json(path: &str, token: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn token_permissions_follow_the_owners_current_role() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let user = app
        .store
        .ensure_user(app.org_id, "demoted@test.local", "Demoted", UserRole::Owner)
        .await
        .unwrap();
    let issued = orbit_security::issue_api_token();
    let scopes: Vec<String> = orbit_security::Permission::all_for(UserRole::Owner)
        .into_iter()
        .map(str::to_string)
        .collect();
    app.store
        .create_api_token(
            app.org_id,
            user.id,
            "explicit",
            &issued.prefix,
            &issued.hash,
            &scopes,
            None,
        )
        .await
        .unwrap();
    let (status, current) = send(
        &app.router,
        auth_get("/api/v1/settings/access", &issued.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(
        &app.router,
        put_json("/api/v1/settings/access", &issued.token, &current),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "owner write allowed: {body}");

    app.store
        .set_user_role(app.org_id, user.id, UserRole::Viewer)
        .await
        .unwrap();

    let (status, _) = send(
        &app.router,
        auth_get("/api/v1/settings/access", &issued.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "viewer can still read");
    let (status, body) = send(
        &app.router,
        put_json("/api/v1/settings/access", &issued.token, &current),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = send(
        &app.router,
        auth_post(
            "/api/v1/api-tokens",
            &issued.token,
            json!({ "name": "escalate" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn tokens_cannot_be_minted_with_scopes_the_caller_lacks() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let user = app
        .store
        .ensure_user(app.org_id, "minter@test.local", "Minter", UserRole::Viewer)
        .await
        .unwrap();
    let role = format!("Token minter {}", Uuid::now_v7());
    app.store
        .create_custom_role(
            app.org_id,
            &role,
            &[
                "agents:read".to_string(),
                "settings:read".to_string(),
                "settings:write".to_string(),
            ],
        )
        .await
        .unwrap();
    app.store
        .set_user_role_name(app.org_id, user.id, &role)
        .await
        .unwrap();
    let token = token_for(&app.store, app.org_id, user.id, "minter").await;

    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/api-tokens",
            &token,
            json!({ "name": "too much", "scopes": ["agents:read", "providers:write", "*"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    let details = body["error"]["details"].to_string();
    assert!(details.contains("providers:write"), "{details}");
    assert!(!details.contains("'*'"), "{details}");
    assert!(!details.contains("'agents:read'"), "{details}");

    // `*` is allowed: it resolves to the caller's current permissions at request time.
    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/api-tokens",
            &token,
            json!({ "name": "wildcard", "scopes": ["*"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/api-tokens",
            &token,
            json!({ "name": "ok", "scopes": ["agents:read"] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, body) = send(
        &app.router,
        auth_post("/api/v1/api-tokens", &token, json!({ "name": "default" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let mut got: Vec<String> = serde_json::from_value(body["scopes"].clone()).unwrap();
    got.sort();
    assert_eq!(got, ["agents:read", "settings:read", "settings:write"]);
}

#[tokio::test]
async fn dashboard_summary_hides_other_teams_agents_and_executions() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let owner = owner_user(&app).await;
    // The demo agent created by the harness belongs to the owner only.
    make_private(&app.store, app.org_id, "agents", app.agent_id, owner.id).await;

    let team_x = app
        .store
        .create_access_team(app.org_id, "Team X", "team-x", Some(owner.id))
        .await
        .unwrap();
    let team_y = app
        .store
        .create_access_team(app.org_id, "Team Y", "team-y", Some(owner.id))
        .await
        .unwrap();
    let viewer = app
        .store
        .ensure_user(
            app.org_id,
            "dash-viewer@test.local",
            "Viewer",
            UserRole::Viewer,
        )
        .await
        .unwrap();
    app.store
        .upsert_team_member(team_x.id, viewer.id, "viewer")
        .await
        .unwrap();
    app.store
        .set_platform_setting(
            "team_isolation",
            &json!({ "enabled": true }),
            Some(owner.id),
        )
        .await
        .unwrap();

    let (_, exec_x) =
        seed_team_agent_with_execution(&app, owner.id, team_x.id, "visible-to-x").await;
    let (_, exec_y) =
        seed_team_agent_with_execution(&app, owner.id, team_y.id, "SECRET-of-team-y").await;
    let viewer_token = token_for(&app.store, app.org_id, viewer.id, "dash-viewer").await;

    let (status, body) = send(
        &app.router,
        auth_get("/api/v1/dashboard/summary", &viewer_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(recent_ids(&body), vec![exec_x.to_string()]);
    assert!(!body.to_string().contains("SECRET-of-team-y"));
    let summary = &body["summary"];
    assert_eq!(summary["totalAgents"], 1);
    assert_eq!(summary["activeAgents"], 1);
    assert_eq!(summary["executions24h"], 1);
    assert_eq!(summary["runningExecutions"], 1);
    assert_eq!(summary["pendingApprovals"], 1);
    assert_eq!(usage_total(&body), 1);

    // The owner still sees everything: both teams' executions and the private demo agent.
    let (status, body) = send(
        &app.router,
        auth_get("/api/v1/dashboard/summary", "test-token"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let recent = recent_ids(&body);
    assert!(recent.contains(&exec_x.to_string()));
    assert!(recent.contains(&exec_y.to_string()));
    let summary = &body["summary"];
    assert_eq!(summary["totalAgents"], 3);
    assert_eq!(summary["executions24h"], 2);
    assert_eq!(summary["pendingApprovals"], 2);
    assert_eq!(usage_total(&body), 2);
}

#[tokio::test]
async fn dashboard_summary_is_empty_for_user_outside_every_team() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let owner = owner_user(&app).await;
    make_private(&app.store, app.org_id, "agents", app.agent_id, owner.id).await;
    let team_y = app
        .store
        .create_access_team(app.org_id, "Team Y2", "team-y2", Some(owner.id))
        .await
        .unwrap();
    let (_, exec_y) = seed_team_agent_with_execution(&app, owner.id, team_y.id, "hidden").await;
    let outsider = app
        .store
        .ensure_user(
            app.org_id,
            "dash-outsider@test.local",
            "Outsider",
            UserRole::Viewer,
        )
        .await
        .unwrap();
    let token = token_for(&app.store, app.org_id, outsider.id, "dash-outsider").await;
    let (status, body) = send(&app.router, auth_get("/api/v1/dashboard/summary", &token)).await;
    assert_eq!(status, StatusCode::OK);
    let summary = &body["summary"];
    assert_eq!(summary["totalAgents"], 0);
    assert_eq!(summary["executions24h"], 0);
    assert_eq!(summary["pendingApprovals"], 0);
    assert!(recent_ids(&body).is_empty());
    assert_eq!(usage_total(&body), 0);
    assert!(!body.to_string().contains(&exec_y.to_string()));
}

async fn owner_user(app: &TestApp) -> orbit_store::models::User {
    app.store
        .list_users(app.org_id)
        .await
        .unwrap()
        .into_iter()
        .find(|user| user.role == "owner")
        .unwrap()
}

fn recent_ids(body: &Value) -> Vec<String> {
    body["summary"]["recentExecutions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

async fn seed_team_agent_with_execution(
    app: &TestApp,
    owner_id: Uuid,
    team_id: Uuid,
    task: &str,
) -> (Uuid, Uuid) {
    let definition: orbit_domain::AgentDefinition = serde_json::from_value(definition()).unwrap();
    let (agent, version) = app
        .store
        .create_agent(app.org_id, Some(owner_id), &definition)
        .await
        .unwrap();
    app.store
        .set_agent_status(app.org_id, agent.id, orbit_domain::AgentStatus::Active)
        .await
        .unwrap();
    app.store
        .set_resource_visibility(app.org_id, "agents", agent.id, "team", Some(team_id))
        .await
        .unwrap();
    let execution = app
        .store
        .create_execution(orbit_store::executions::NewExecution {
            org_id: app.org_id,
            agent_id: agent.id,
            agent_version_id: version.id,
            agent_name: &agent.name,
            harness_kind: orbit_domain::HarnessKind::Mock,
            trigger_source: orbit_domain::TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input: json!({ "task": task }),
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(owner_id),
            idempotency_key: None,
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();
    app.store
        .create_approval(app.org_id, execution.id, json!({}), None)
        .await
        .unwrap();
    (agent.id, execution.id)
}

fn usage_total(body: &Value) -> i64 {
    body["usage"]["days"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["executions"].as_i64().unwrap())
        .sum()
}

#[tokio::test]
async fn trigger_creation_requires_use_on_the_target_agent() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let alice = app
        .store
        .ensure_user(
            app.org_id,
            "trigger-alice@test.local",
            "Alice",
            UserRole::Owner,
        )
        .await
        .unwrap();
    let bob = app
        .store
        .ensure_user(
            app.org_id,
            "trigger-bob@test.local",
            "Bob",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let agent = create_demo_agent(&app.store, app.org_id, alice.id).await;
    make_private(&app.store, app.org_id, "agents", agent, alice.id).await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "trigger-bob").await;
    let alice_token = token_for(&app.store, app.org_id, alice.id, "trigger-alice").await;

    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/triggers",
            &bob_token,
            json!({ "agentId": agent, "kind": "manual" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/triggers",
            &alice_token,
            json!({ "agentId": agent, "kind": "manual" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["agentId"], agent.to_string());
}

#[tokio::test]
async fn team_queue_requires_team_view_and_use() {
    let Some(app) = test_app(false).await else {
        return;
    };
    let alice = app
        .store
        .ensure_user(
            app.org_id,
            "queue-alice@test.local",
            "Alice",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let bob = app
        .store
        .ensure_user(
            app.org_id,
            "queue-bob@test.local",
            "Bob",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let charlie = app
        .store
        .ensure_user(
            app.org_id,
            "queue-charlie@test.local",
            "Charlie",
            UserRole::Operator,
        )
        .await
        .unwrap();
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;
    let charlie_token = token_for(&app.store, app.org_id, charlie.id, "charlie").await;

    let team = app
        .store
        .create_agent_team(
            app.org_id,
            "Queue team",
            "",
            &json!({ "steps": [] }),
            Some(alice.id),
        )
        .await
        .unwrap();
    app.store
        .assign_resource_owner(app.org_id, "agent_teams", team.id, alice.id)
        .await
        .unwrap();
    let access_team = app
        .store
        .create_access_team(
            app.org_id,
            "Queue viewers",
            &format!("queue-{}", Uuid::now_v7()),
            Some(alice.id),
        )
        .await
        .unwrap();
    app.store
        .upsert_team_member(access_team.id, bob.id, "viewer")
        .await
        .unwrap();
    app.store
        .set_resource_visibility(
            app.org_id,
            "agent_teams",
            team.id,
            "team",
            Some(access_team.id),
        )
        .await
        .unwrap();

    // A non-member cannot see the queue at all.
    let (status, _) = send(
        &app.router,
        auth_get(
            &format!("/api/v1/agent-teams/{}/queue", team.id),
            &charlie_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agent-teams/{}/queue", team.id),
            &charlie_token,
            json!({ "items": [{ "externalKey": "x" }] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A team viewer can read the queue but cannot enqueue or run.
    let (status, _) = send(
        &app.router,
        auth_get(
            &format!("/api/v1/agent-teams/{}/queue", team.id),
            &bob_token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agent-teams/{}/queue", team.id),
            &bob_token,
            json!({ "items": [{ "externalKey": "x" }] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agent-teams/{}/queue", team.id),
            &alice_token,
            json!({ "items": [{ "externalKey": "item-1" }] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let item_id = body[0]["id"].as_str().unwrap();
    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agent-teams/{}/queue/{item_id}/run", team.id),
            &bob_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
