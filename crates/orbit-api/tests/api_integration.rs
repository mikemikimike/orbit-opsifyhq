use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use http_body_util::BodyExt;
use orbit_api::{build_router, AppState, DevIdentity};
use orbit_domain::UserRole;
use orbit_events::MemoryEventBus;
use orbit_harness::HarnessRegistry;
use orbit_security::{AuthConfig, AuthMode, DevAuthConfig, SecretCipher};
use orbit_store::Store;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

struct TestApp {
    state: AppState,
    router: axum::Router,
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

    let bus = Arc::new(MemoryEventBus::new(128));
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

<
async fn send_raw(
    router: &axum::Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = router.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

#[tokio::test]
async fn docs_index_lists_categories_and_excludes_planning() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let (status, body) = send(&app.router, get("/api/v1/docs")).await;
    assert_eq!(status, StatusCode::OK);
    let categories = body["categories"].as_array().expect("categories");
    assert!(categories.len() >= 4);
    let ids: Vec<&str> = categories.iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.contains(&"guide") && ids.contains(&"architecture"));
    let slugs: Vec<&str> = categories
        .iter()
        .flat_map(|c| c["pages"].as_array().unwrap())
        .filter_map(|p| p["slug"].as_str())
        .collect();
    assert!(slugs.contains(&"architecture/overview"));
    assert!(!slugs.iter().any(|s| s.starts_with("planning")));
    let first = &categories[0]["pages"][0];
    assert!(first["title"].as_str().is_some_and(|t| !t.is_empty()));
    assert!(first["url"].as_str().unwrap().starts_with("/api/v1/docs/"));

    let (status, headers, bytes) = send_raw(&app.router, get("/api/v1/docs?format=html")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(String::from_utf8_lossy(&bytes).contains("architecture/overview"));
}

#[tokio::test]
async fn docs_page_rewrites_links_and_supports_html() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let (status, body) = send(&app.router, get("/api/v1/docs/index")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["slug"], "index");
    let markdown = body["markdown"].as_str().unwrap();
    assert!(
        markdown.contains("](/api/v1/docs/architecture/overview)"),
        "{markdown}"
    );
    assert!(!markdown.contains("overview.md)"));

    let (status, headers, bytes) = send_raw(
        &app.router,
        get("/api/v1/docs/architecture/overview?format=markdown"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/markdown"));
    assert!(String::from_utf8_lossy(&bytes).starts_with("# "));

    let request = Request::builder()
        .uri("/api/v1/docs/index")
        .header("accept", "text/html")
        .body(Body::empty())
        .unwrap();
    let (status, headers, bytes) = send_raw(&app.router, request).await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert!(headers.contains_key("content-security-policy"));
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("<h1 id="));
    assert!(html.contains("href=\"/api/v1/docs/"));
}

#[tokio::test]
async fn docs_assets_are_served_with_content_type() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let (status, headers, bytes) = send_raw(
        &app.router,
        get("/api/v1/docs/assets/architecture/orbit-overview.png"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/png");
    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));

    let (status, headers, _) = send_raw(
        &app.router,
        get("/api/v1/docs/assets/architecture/orbit-overview.svg"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/svg+xml");

    // A page whose slug starts with `assets/` is still reachable.
    let (status, body) = send(&app.router, get("/api/v1/docs/assets/brand")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["slug"], "assets/brand");
}

#[tokio::test]
async fn docs_reject_traversal_unknown_and_planning() {
    let Some(app) = test_app(true).await else {
        return;
    };
    for path in [
        "/api/v1/docs/../Cargo.toml",
        "/api/v1/docs/..%2F..%2FCargo.toml",
        "/api/v1/docs/%2e%2e/%2e%2e/Cargo.toml",
        "/api/v1/docs/assets/../../Cargo.toml",
        "/api/v1/docs/assets/..%2F..%2FCargo.toml",
        "/api/v1/docs/does/not/exist",
        "/api/v1/docs/planning/provider-ux-and-harness-decoupling",
        "/api/v1/docs/assets/planning/provider-ux-and-harness-decoupling.md",
        "/api/v1/docs/assets/nope.png",
    ] {
        let (status, body) = send(&app.router, get(path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body["error"]["code"], "not_found", "{path}");
        assert!(!body["error"]["requestId"].as_str().unwrap_or("").is_empty());
    }
}

#[tokio::test]
async fn docs_require_authentication_but_swagger_stays_public() {
    let Some(app) = test_app(false).await else {
        return;
    };
    for path in [
        "/api/v1/docs",
        "/api/v1/docs/index",
        "/api/v1/docs/assets/architecture/orbit-overview.png",
    ] {
        let (status, body) = send(&app.router, get(path)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(body["error"]["code"], "unauthorized");
    }
    let (status, _) = send(&app.router, auth_get("/api/v1/docs", "test-token")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(&app.router, get("/api/v1/openapi.json")).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(&app.router, get("/api/v1/meta")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["docsUrl"], "/api/v1/docs");
}
