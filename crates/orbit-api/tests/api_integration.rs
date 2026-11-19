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

// ---------------------------------------------------------------------------
// Issue #140: shared execution preflight and related permission checks
// ---------------------------------------------------------------------------

async fn token_with_scopes(
    store: &Store,
    org_id: Uuid,
    user_id: Uuid,
    name: &str,
    scopes: &[&str],
) -> String {
    let issued = orbit_security::issue_api_token();
    let scopes: Vec<String> = scopes.iter().map(|scope| (*scope).to_string()).collect();
    store
        .create_api_token(
            org_id,
            user_id,
            name,
            &issued.prefix,
            &issued.hash,
            &scopes,
            None,
        )
        .await
        .expect("api token");
    issued.token
}

fn auth_json(method: &str, path: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn webhook_connection(app: &TestApp, name: &str, owner: Uuid) -> Uuid {
    let connector = app
        .store
        .get_connector_by_slug(app.org_id, "webhook")
        .await
        .unwrap()
        .expect("webhook connector");
    let connection = app
        .store
        .create_connection(
            app.org_id,
            connector.id,
            name,
            &json!({ "url": "http://127.0.0.1:9/hook" }),
            &[],
            None,
            Some(owner),
        )
        .await
        .unwrap();
    make_private(&app.store, app.org_id, "connections", connection.id, owner).await;
    connection.id
}

/// Agent owned by `owner` with optional connection binding and hourly quota.
async fn bound_agent(
    app: &TestApp,
    owner: Uuid,
    connection: Option<Uuid>,
    max_per_hour: Option<u32>,
) -> Uuid {
    let mut value = definition();
    if let Some(connection) = connection {
        value["spec"]["connectors"] =
            json!([{ "connector": "webhook", "connectionId": connection }]);
    }
    if let Some(max) = max_per_hour {
        value["spec"]["autonomy"]["maxExecutionsPerHour"] = json!(max);
    }
    let definition: orbit_domain::AgentDefinition = serde_json::from_value(value).unwrap();
    let (agent, _) = app
        .store
        .create_agent(app.org_id, Some(owner), &definition)
        .await
        .unwrap();
    app.store
        .set_agent_status(app.org_id, agent.id, orbit_domain::AgentStatus::Active)
        .await
        .unwrap();
    app.store
        .assign_resource_owner(app.org_id, "agents", agent.id, owner)
        .await
        .unwrap();
    agent.id
}

async fn share_org(app: &TestApp, table: &str, id: Uuid) {
    app.store
        .set_resource_visibility(app.org_id, table, id, "org", None)
        .await
        .unwrap();
}

async fn operator(app: &TestApp, email: &str) -> orbit_store::User {
    app.store
        .ensure_user(app.org_id, email, email, UserRole::Operator)
        .await
        .unwrap()
}

async fn access_team(app: &TestApp, slug: &str) -> Uuid {
    app.store
        .create_access_team(app.org_id, slug, slug, None)
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn agent_specs_require_use_of_referenced_connections_and_providers() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = operator(&app, "pf-spec-alice@test.local").await;
    let bob = operator(&app, "pf-spec-bob@test.local").await;
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;
    let alice_connection = webhook_connection(&app, "Alice hook", alice.id).await;

    let mut with_connection = definition();
    with_connection["spec"]["connectors"] =
        json!([{ "connector": "webhook", "connectionId": alice_connection }]);

    // Invisible connection: 404 with the field path in the envelope details.
    let (status, body) = send(
        &app.router,
        auth_post("/api/v1/agents", &bob_token, with_connection.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body["error"]["details"][0]["path"],
        "spec.connectors[0].connectionId"
    );

    // The owner can bind it.
    let (status, body) = send(
        &app.router,
        auth_post("/api/v1/agents", &alice_token, with_connection.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Updating an existing agent re-checks the references.
    let bob_agent = bound_agent(&app, bob.id, None, None).await;
    let (status, _) = send(
        &app.router,
        auth_patch(
            &format!("/api/v1/agents/{bob_agent}"),
            &bob_token,
            json!({ "definition": with_connection.clone() }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Visible but not usable (team viewer): 403.
    let team = access_team(&app, "pf-spec-team").await;
    app.store
        .upsert_team_member(team, bob.id, "viewer")
        .await
        .unwrap();
    app.store
        .set_resource_visibility(
            app.org_id,
            "connections",
            alice_connection,
            "team",
            Some(team),
        )
        .await
        .unwrap();
    let (status, _) = send(
        &app.router,
        auth_post("/api/v1/agents", &bob_token, with_connection),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Providers: a private provider cannot be bound by another user.
    let provider = app
        .store
        .create_provider(
            app.org_id,
            "pf-alice-provider",
            "openai_compatible",
            &json!({ "baseUrl": "http://127.0.0.1:9/v1" }),
            None,
        )
        .await
        .unwrap();
    make_private(&app.store, app.org_id, "providers", provider.id, alice.id).await;
    let mut with_provider = definition();
    with_provider["spec"]["model"] =
        json!({ "management": "platform", "provider": "pf-alice-provider", "model": "m" });
    let (status, body) = send(
        &app.router,
        auth_post("/api/v1/agents", &bob_token, with_provider.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["details"][0]["path"], "spec.model.provider");
    let (status, body) = send(
        &app.router,
        auth_post("/api/v1/agents", &alice_token, with_provider),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn rerun_applies_the_execute_preflight() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = operator(&app, "pf-rerun-alice@test.local").await;
    let bob = operator(&app, "pf-rerun-bob@test.local").await;
    let bob_token = token_for(&app.store, app.org_id, bob.id, "bob").await;
    // Alice owns the connection and shares it org-wide; bob's agent binds it.
    let connection = webhook_connection(&app, "Shared hook", alice.id).await;
    share_org(&app, "connections", connection).await;
    let agent = bound_agent(&app, bob.id, Some(connection), None).await;

    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agents/{agent}/executions"),
            &bob_token,
            json!({ "task": "first" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let execution_id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    app.store
        .request_execution_cancel(app.org_id, execution_id)
        .await
        .unwrap();

    // Baseline: rerun works while bob can use the connection.
    let rerun = format!("/api/v1/executions/{execution_id}/rerun");
    let (status, body) = send(&app.router, auth_post(&rerun, &bob_token, json!({}))).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let second: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    app.store
        .request_execution_cancel(app.org_id, second)
        .await
        .unwrap();

    // Connection access revoked (alice makes it private again): the rerun is refused.
    make_private(&app.store, app.org_id, "connections", connection, alice.id).await;
    let (status, body) = send(&app.router, auth_post(&rerun, &bob_token, json!({}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(
        body["error"]["details"][0]["path"],
        "spec.connectors[0].connectionId"
    );

    // Quota: maxExecutionsPerHour is enforced with 429.
    let limited = bound_agent(&app, bob.id, None, Some(1)).await;
    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agents/{limited}/executions"),
            &bob_token,
            json!({ "task": "once" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let first: Uuid = body["id"].as_str().unwrap().parse().unwrap();
    app.store
        .request_execution_cancel(app.org_id, first)
        .await
        .unwrap();
    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/executions/{first}/rerun"),
            &bob_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");

    // A harness that cannot run executions blocks the rerun.
    let planned = HarnessRegistry::builtin()
        .descriptors()
        .into_iter()
        .find(|descriptor| descriptor.kind == orbit_domain::HarnessKind::ClaudeCode)
        .expect("claude code descriptor");
    assert_ne!(planned.implementation_status.as_str(), "available");
    app.store.upsert_harness_descriptor(&planned).await.unwrap();
    let plain = bound_agent(&app, bob.id, None, None).await;
    let agent_row = app.store.get_agent(app.org_id, plain).await.unwrap();
    let version = app
        .store
        .get_current_version(app.org_id, &agent_row)
        .await
        .unwrap();
    let blocked = app
        .store
        .create_execution(orbit_store::executions::NewExecution {
            org_id: app.org_id,
            agent_id: plain,
            agent_version_id: version.id,
            agent_name: &agent_row.name,
            harness_kind: orbit_domain::HarnessKind::ClaudeCode,
            trigger_source: orbit_domain::TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input: json!({ "task": "x" }),
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(bob.id),
            idempotency_key: None,
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();
    app.store
        .request_execution_cancel(app.org_id, blocked.id)
        .await
        .unwrap();
    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/executions/{}/rerun", blocked.id),
            &bob_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["details"][0]["path"], "harness");
}

#[tokio::test]
async fn cancel_and_team_runs_require_use_not_just_view() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = operator(&app, "pf-use-alice@test.local").await;
    let viola = operator(&app, "pf-use-viola@test.local").await;
    let mark = operator(&app, "pf-use-mark@test.local").await;
    let alice_token = token_for(&app.store, app.org_id, alice.id, "alice").await;
    let viola_token = token_for(&app.store, app.org_id, viola.id, "viola").await;
    let mark_token = token_for(&app.store, app.org_id, mark.id, "mark").await;
    let team = access_team(&app, "pf-use-team").await;
    app.store
        .upsert_team_member(team, viola.id, "viewer")
        .await
        .unwrap();
    app.store
        .upsert_team_member(team, mark.id, "member")
        .await
        .unwrap();

    let agent = bound_agent(&app, alice.id, None, None).await;
    app.store
        .set_resource_visibility(app.org_id, "agents", agent, "team", Some(team))
        .await
        .unwrap();
    let (status, body) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agents/{agent}/executions"),
            &alice_token,
            json!({ "task": "run" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let execution = body["id"].as_str().unwrap().to_string();

    // A team viewer can see the execution but not cancel it.
    let (status, _) = send(
        &app.router,
        auth_get(&format!("/api/v1/executions/{execution}"), &viola_token),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/executions/{execution}/cancel"),
            &viola_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/executions/{execution}/cancel"),
            &mark_token,
            json!({}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Team runs: viewer refused, member allowed.
    let team_definition = json!({ "steps": [{ "agentId": agent }] });
    let team_row = app
        .store
        .create_agent_team(app.org_id, "Use team", "", &team_definition, Some(alice.id))
        .await
        .unwrap();
    app.store
        .assign_resource_owner(app.org_id, "agent_teams", team_row.id, alice.id)
        .await
        .unwrap();
    app.store
        .set_resource_visibility(app.org_id, "agent_teams", team_row.id, "team", Some(team))
        .await
        .unwrap();
    let runs = format!("/api/v1/agent-teams/{}/runs", team_row.id);
    let (status, _) = send(
        &app.router,
        auth_post(&runs, &viola_token, json!({ "task": "go" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A caller-supplied repository outside the team's configured set is refused.
    let (status, body) = send(
        &app.router,
        auth_post(
            &runs,
            &mark_token,
            json!({ "task": "go", "payload": { "repository": "evil/repo" } }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(body["error"]["details"][0]["path"], "payload.repository");

    let (status, body) = send(
        &app.router,
        auth_post(&runs, &mark_token, json!({ "task": "go" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // A step agent bound to a connection the member cannot use blocks the run.
    let secret = webhook_connection(&app, "Alice private hook", alice.id).await;
    let bound = bound_agent(&app, alice.id, Some(secret), None).await;
    app.store
        .set_resource_visibility(app.org_id, "agents", bound, "team", Some(team))
        .await
        .unwrap();
    let team2 = app
        .store
        .create_agent_team(
            app.org_id,
            "Bound team",
            "",
            &json!({ "steps": [{ "agentId": bound }] }),
            Some(alice.id),
        )
        .await
        .unwrap();
    app.store
        .set_resource_visibility(app.org_id, "agent_teams", team2.id, "team", Some(team))
        .await
        .unwrap();
    let (status, _) = send(
        &app.router,
        auth_post(
            &format!("/api/v1/agent-teams/{}/runs", team2.id),
            &mark_token,
            json!({ "task": "go" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tool_policies_require_manage_on_the_target_agent() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = operator(&app, "pf-policy-alice@test.local").await;
    let carol = operator(&app, "pf-policy-carol@test.local").await;
    let carol_token = token_with_scopes(
        &app.store,
        app.org_id,
        carol.id,
        "carol",
        &["settings:write", "settings:read", "agents:read"],
    )
    .await;
    let alice_agent = bound_agent(&app, alice.id, None, None).await;
    make_private(&app.store, app.org_id, "agents", alice_agent, alice.id).await;
    let carol_agent = bound_agent(&app, carol.id, None, None).await;
    let policy = |agent: Option<Uuid>| {
        json!({
            "name": "allow bash",
            "agentId": agent,
            "action": "allow",
            "matcher": { "tool": "bash" }
        })
    };

    let (status, _) = send(
        &app.router,
        auth_post(
            "/api/v1/tool-policies",
            &carol_token,
            policy(Some(alice_agent)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, body) = send(
        &app.router,
        auth_post(
            "/api/v1/tool-policies",
            &carol_token,
            policy(Some(carol_agent)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let policy_id = body["id"].as_str().unwrap().to_string();

    // Moving the policy onto an agent she cannot manage is refused too.
    let (status, _) = send(
        &app.router,
        auth_json(
            "PATCH",
            &format!("/api/v1/tool-policies/{policy_id}"),
            &carol_token,
            policy(Some(alice_agent)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Org-wide policies keep requiring only settings:write.
    let (status, body) = send(
        &app.router,
        auth_post("/api/v1/tool-policies", &carol_token, policy(None)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn export_checks_the_read_permission_of_the_resource_kind() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let dave = operator(&app, "pf-export-dave@test.local").await;
    let agents_only = token_with_scopes(
        &app.store,
        app.org_id,
        dave.id,
        "agents-only",
        &["agents:read"],
    )
    .await;
    let dave_agent = bound_agent(&app, dave.id, None, None).await;
    let provider = app
        .store
        .create_provider(
            app.org_id,
            "pf-export-provider",
            "openai_compatible",
            &json!({ "baseUrl": "http://127.0.0.1:9/v1" }),
            None,
        )
        .await
        .unwrap();
    share_org(&app, "providers", provider.id).await;

    let (status, _) = send(
        &app.router,
        auth_get(
            &format!("/api/v1/export/providers/{}", provider.id),
            &agents_only,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = send(
        &app.router,
        auth_get(&format!("/api/v1/export/agents/{dave_agent}"), &agents_only),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let providers_reader = token_with_scopes(
        &app.store,
        app.org_id,
        dave.id,
        "providers",
        &["agents:read", "providers:read"],
    )
    .await;
    let (status, _) = send(
        &app.router,
        auth_get(
            &format!("/api/v1/export/providers/{}", provider.id),
            &providers_reader,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn scheduled_trigger_fails_when_its_owner_lost_connection_access() {
    let Some(app) = test_app(true).await else {
        return;
    };
    let alice = operator(&app, "pf-trigger-alice@test.local").await;
    let bob = operator(&app, "pf-trigger-bob@test.local").await;
    // Bob owns the connection and shares it org-wide; alice's agent and trigger use it.
    let connection = webhook_connection(&app, "Bob trigger hook", bob.id).await;
    share_org(&app, "connections", connection).await;
    let agent = bound_agent(&app, alice.id, Some(connection), None).await;
    let trigger = app
        .store
        .create_trigger(
            app.org_id,
            agent,
            "schedule",
            None,
            json!({ "cron": "* * * * *" }),
            true,
        )
        .await
        .unwrap();
    app.store
        .assign_resource_owner(app.org_id, "triggers", trigger.id, alice.id)
        .await
        .unwrap();

    // While the owner can use the connection the trigger runs.
    let current = app.store.get_trigger(app.org_id, trigger.id).await.unwrap();
    orbit_api::background::fire_trigger(&app.state, current)
        .await
        .expect("first fire succeeds");
    let (executions, _, _) = app
        .store
        .list_executions(app.org_id, 50, None, None, Some(agent), None)
        .await
        .unwrap();
    assert_eq!(executions.len(), 1);
    app.store
        .request_execution_cancel(app.org_id, executions[0].id)
        .await
        .unwrap();

    // Bob withdraws the connection: the trigger owner has lost access at fire time.
    make_private(&app.store, app.org_id, "connections", connection, bob.id).await;
    let current = app.store.get_trigger(app.org_id, trigger.id).await.unwrap();
    let error = orbit_api::background::fire_trigger(&app.state, current)
        .await
        .expect_err("fire is refused");
    assert!(error.to_string().contains("refused"), "{error}");
    let (executions, _, _) = app
        .store
        .list_executions(app.org_id, 50, None, None, Some(agent), None)
        .await
        .unwrap();
    assert_eq!(executions.len(), 1, "no execution may be created");
    let (logs, _) = app
        .store
        .list_audit_logs(app.org_id, 10, None, Some("trigger.fire_denied"))
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
}
