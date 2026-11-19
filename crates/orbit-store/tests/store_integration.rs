use std::sync::Arc;

use base64::Engine;
use orbit_domain::{
    AgentDefinition, AgentMetadata, AgentSpec, AgentStatus, AutonomySpec, ExecutionMode,
    ExecutionStatus, HarnessKind, HarnessSpec, Instructions, LogLevel, MemorySpec, ModelSpec,
    ResourceLimits, TriggerSource, TriggerSpec, UserRole,
};
use orbit_security::{SecretCipher, SecretValue};
use orbit_store::executions::NewExecution;
use orbit_store::Store;

async fn test_store() -> Option<Store> {
    let url = std::env::var("ORBIT_TEST_DATABASE_URL").ok()?;
    let key = base64::engine::general_purpose::STANDARD.encode([7u8; 32]);
    let cipher = Arc::new(SecretCipher::from_base64_key(&key).unwrap());
    let store = Store::connect(&url, 5)
        .await
        .expect("connect")
        .with_cipher(cipher);
    store.migrate().await.expect("migrate");
    Some(store)
}

async fn seed_org(store: &Store) -> (uuid::Uuid, uuid::Uuid) {
    let slug = format!("test-{}", uuid::Uuid::now_v7());
    let org = store
        .create_organization(&slug, "Test Org", orbit_domain::OrganizationMode::Personal)
        .await
        .expect("org");
    let user = store
        .ensure_user(org.id, "owner@test.local", "Test Owner", UserRole::Owner)
        .await
        .expect("user");
    (org.id, user.id)
}

fn definition(name: &str) -> AgentDefinition {
    AgentDefinition {
        api_version: orbit_domain::AGENT_API_VERSION.into(),
        kind: orbit_domain::AGENT_KIND.into(),
        metadata: AgentMetadata {
            name: name.into(),
            description: "integration test agent".into(),
            labels: Default::default(),
        },
        spec: AgentSpec {
            harness: Some(HarnessSpec {
                kind: HarnessKind::Mock,
                execution_mode: ExecutionMode::OnDemand,
                placement: orbit_domain::PlacementTarget::Auto,
                config: serde_json::json!({}),
            }),
            harness_config: Default::default(),
            model: ModelSpec::default(),
            instructions: Instructions {
                system: "You are a test agent.".into(),
                constraints: vec![],
            },
            connectors: vec![],
            skills: vec![],
            triggers: vec![TriggerSpec::Schedule {
                cron: "*/15 * * * *".into(),
                timezone: None,
                task: None,
            }],
            autonomy: AutonomySpec {
                level: 2,
                approval_mode: Default::default(),
                max_executions_per_hour: Some(10),
            },
            resources: ResourceLimits::default(),
            memory: MemorySpec::default(),
            run_defaults: Default::default(),
        },
    }
}

#[tokio::test]
async fn agent_lifecycle_and_versions() {
    let Some(store) = test_store().await else {
        eprintln!("skipping: ORBIT_TEST_DATABASE_URL not set");
        return;
    };
    let (org, user) = seed_org(&store).await;

    let (agent, version) = store
        .create_agent(org, Some(user), &definition("Lifecycle Agent"))
        .await
        .expect("create agent");
    assert_eq!(version.version, 1);
    assert_eq!(agent.current_version, 1);
    assert_eq!(agent.status, "draft");
    assert_eq!(agent.slug, "lifecycle-agent");

    // Schedule trigger was materialized from the spec.
    let triggers = store
        .list_triggers(org, Some(agent.id), None)
        .await
        .unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].kind, "schedule");
    assert!(triggers[0].next_run_at.is_some());

    // Duplicate version is rejected.
    let err = store
        .add_agent_version(org, agent.id, Some(user), &definition("Lifecycle Agent"))
        .await
        .expect_err("duplicate spec must conflict");
    assert!(matches!(err, orbit_store::StoreError::Conflict(_)));

    // A changed spec creates version 2 and updates metadata.
    let mut changed = definition("Lifecycle Agent v2");
    changed.spec.autonomy.level = 3;
    let (agent, v2) = store
        .add_agent_version(org, agent.id, Some(user), &changed)
        .await
        .expect("new version");
    assert_eq!(v2.version, 2);
    assert_eq!(agent.current_version, 2);
    assert_eq!(agent.name, "Lifecycle Agent v2");

    let (rows, _, total) = store
        .list_agents(org, 10, None, None, Some("Lifecycle"), None)
        .await
        .unwrap();
    assert_eq!(total, 1);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].spec.is_some());

    // Lifecycle transitions
    let active = store
        .set_agent_status(org, agent.id, AgentStatus::Active)
        .await
        .unwrap();
    assert_eq!(active.status, "active");
    let paused = store
        .set_agent_status(org, agent.id, AgentStatus::Paused)
        .await
        .unwrap();
    assert_eq!(paused.status, "paused");
    assert!(store
        .set_agent_status(org, agent.id, AgentStatus::Paused)
        .await
        .is_err());
    assert_eq!(
        store
            .list_agent_versions(org, agent.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn execution_lifecycle_and_events() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;
    let (agent, version) = store
        .create_agent(org, Some(user), &definition("Execution Agent"))
        .await
        .unwrap();

    let input = serde_json::json!({
        "task": "hello",
        "payload": {},
        "provenance": { "source": "test", "trusted": true, "receivedAt": chrono::Utc::now() }
    });
    let execution = store
        .create_execution(NewExecution {
            org_id: org,
            agent_id: agent.id,
            agent_version_id: version.id,
            agent_name: &agent.name,
            harness_kind: HarnessKind::Mock,
            trigger_source: TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input: input.clone(),
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(user),
            idempotency_key: Some("idem-1"),
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();
    assert_eq!(execution.status, "queued");

    // Idempotent replay returns the same row.
    let replay = store
        .create_execution(NewExecution {
            org_id: org,
            agent_id: agent.id,
            agent_version_id: version.id,
            agent_name: &agent.name,
            harness_kind: HarnessKind::Mock,
            trigger_source: TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input,
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(user),
            idempotency_key: Some("idem-1"),
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();
    assert_eq!(replay.id, execution.id);

    // Competing claims: only one worker wins.
    let first = store
        .claim_execution(execution.id, "worker-a")
        .await
        .unwrap();
    let second = store
        .claim_execution(execution.id, "worker-b")
        .await
        .unwrap();
    assert!(first.is_some());
    assert!(second.is_none());

    store
        .mark_execution_running(execution.id, Some("session-1"))
        .await
        .unwrap();
    let event = store
        .append_execution_event(
            org,
            execution.id,
            "log",
            orbit_domain::LogLevel::Info,
            "working",
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    assert_eq!(event.seq, 1);
    let second_event = store
        .append_execution_event(
            org,
            execution.id,
            "output",
            orbit_domain::LogLevel::Info,
            "done",
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    assert_eq!(second_event.seq, 2);

    let finished = store
        .finish_execution(
            execution.id,
            ExecutionStatus::Succeeded,
            Some(&serde_json::json!({ "ok": true })),
            None,
            Some(&serde_json::json!({ "outputTokens": 3 })),
            Some("session-1"),
        )
        .await
        .unwrap()
        .expect("finished");
    assert_eq!(finished.status, "succeeded");
    assert!(finished.finished_at.is_some());

    // Terminal executions cannot be re-finished or cancelled.
    assert!(store
        .finish_execution(
            execution.id,
            ExecutionStatus::Failed,
            None,
            None,
            None,
            None
        )
        .await
        .unwrap()
        .is_none());
    assert!(store
        .request_execution_cancel(org, execution.id)
        .await
        .is_err());

    let events = store
        .list_execution_events(org, execution.id, 0, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);

    let summary = store.dashboard_summary(org).await.unwrap();
    assert_eq!(summary.total_agents, 1);
    assert!(summary.executions_24h >= 1);
}

#[tokio::test]
async fn worker_tokens_are_hashed_scoped_and_revocable() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, _user) = seed_org(&store).await;
    let issued = orbit_security::issue_worker_token();
    assert!(issued.token.starts_with("orbit_wk_"));
    let record = store
        .create_worker_token(
            org,
            None,
            "test worker",
            &issued.prefix,
            &issued.hash,
            &["mock".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(record.harnesses, vec!["mock".to_string()]);
    let found = store
        .find_worker_token_by_hash(&issued.hash)
        .await
        .unwrap()
        .expect("token should resolve by hash");
    assert_eq!(found.id, record.id);
    store.revoke_worker_token(org, record.id).await.unwrap();
    assert!(store
        .find_worker_token_by_hash(&issued.hash)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn secrets_are_encrypted_and_org_scoped_entities_work() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;

    let reference = "provider/test-secret";
    store
        .put_secret(
            Some(org),
            reference,
            &SecretValue::new("sk-super-secret"),
            "test",
        )
        .await
        .unwrap();
    let value = store.get_secret_value(reference).await.unwrap().unwrap();
    assert_eq!(value.expose(), "sk-super-secret");

    // Raw ciphertext must not contain the plaintext.
    let ciphertext: String =
        sqlx::query_scalar("SELECT ciphertext FROM secrets WHERE reference = $1")
            .bind(reference)
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert!(!ciphertext.contains("sk-super-secret"));
    assert!(ciphertext.starts_with("v1:"));

    let provider = store
        .create_provider(
            org,
            "Gateway",
            "openai_compatible",
            &serde_json::json!({ "baseUrl": "http://localhost:8000/v1", "model": "qwen" }),
            Some(reference),
        )
        .await
        .unwrap();
    assert_eq!(provider.name, "Gateway");
    store
        .update_provider(org, provider.id, Some("Gateway 2"), None, None, Some(false))
        .await
        .unwrap();
    let providers = store.list_providers(org, None).await.unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].name, "Gateway 2");
    assert!(!providers[0].enabled);

    let harness_provider = store
        .create_provider(
            org,
            "Opencode Go",
            "harness_managed",
            &serde_json::json!({
                "harness": "opencode",
                "provider": "opencode-go",
                "model": "deepseek-v4-pro"
            }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(harness_provider.kind, "harness_managed");

    // Connections
    let manifest = orbit_contracts::ConnectorManifest {
        slug: format!("test-connector-{}", uuid::Uuid::now_v7()),
        name: "Test".into(),
        version: "0.1.0".into(),
        description: String::new(),
        auth: vec![],
        webhooks: false,
        capabilities: vec![],
        config_schema: serde_json::json!({}),
        docs_url: None,
        builtin: false,
    };
    let connector = store.upsert_connector_manifest(&manifest).await.unwrap();
    let connection = store
        .create_connection(
            org,
            connector.id,
            "Primary",
            &serde_json::json!({}),
            &["notes.list".to_string()],
            Some(reference),
            Some(user),
        )
        .await
        .unwrap();
    let rows = store.list_connections(org, None).await.unwrap();
    assert!(rows
        .iter()
        .any(|row| row.connection.id == connection.id && row.connector_slug == manifest.slug));
    store.delete_connection(org, connection.id).await.unwrap();
    assert!(store.list_connections(org, None).await.unwrap().is_empty());
}

#[tokio::test]
async fn org_scoping_is_enforced() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org_a, user_a) = seed_org(&store).await;
    let (org_b, _) = seed_org(&store).await;
    let (agent, _) = store
        .create_agent(org_a, Some(user_a), &definition("Scoped Agent"))
        .await
        .unwrap();

    assert!(store.get_agent(org_a, agent.id).await.is_ok());
    assert!(matches!(
        store.get_agent(org_b, agent.id).await,
        Err(orbit_store::StoreError::NotFound(_))
    ));
    assert!(!store
        .list_triggers(org_b, Some(agent.id), None)
        .await
        .unwrap()
        .iter()
        .any(|t| t.agent_id == agent.id));
}

#[tokio::test]
async fn harness_catalog_upsert_is_idempotent() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, _) = seed_org(&store).await;
    let descriptor = orbit_harness::HarnessRegistry::builtin()
        .descriptors()
        .into_iter()
        .find(|d| d.kind == HarnessKind::Mock)
        .unwrap();
    store.upsert_harness_descriptor(&descriptor).await.unwrap();
    store.upsert_harness_descriptor(&descriptor).await.unwrap();
    let harnesses = store.list_harnesses(org).await.unwrap();
    assert_eq!(
        harnesses.iter().filter(|r| r.kind == "mock").count(),
        1,
        "upsert must not duplicate built-in harnesses"
    );
}

#[tokio::test]
async fn orphaned_executions_are_failed_and_approvals_expired() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;
    let (agent, version) = store
        .create_agent(org, Some(user), &definition("Reaper Agent"))
        .await
        .unwrap();

    let make_execution = |key: &'static str| {
        let store = &store;
        let agent = &agent;
        let version = &version;
        async move {
            store
                .create_execution(NewExecution {
                    org_id: org,
                    agent_id: agent.id,
                    agent_version_id: version.id,
                    agent_name: &agent.name,
                    harness_kind: HarnessKind::Mock,
                    trigger_source: TriggerSource::Manual,
                    placement: "auto",
                    trigger_id: None,
                    input: serde_json::json!({ "task": "x", "payload": {}, "provenance": { "source": "test", "trusted": true, "receivedAt": chrono::Utc::now() } }),
                    session_ref: None,
                    skills: &[],
                    timeout_seconds: 30,
                    requested_by: Some(user),
                    idempotency_key: Some(key),
                    team_run_id: None,
                    team_step_index: None,
                })
                .await
                .unwrap()
        }
    };

    let orphan = make_execution("orphan-1").await;
    store
        .claim_execution(orphan.id, "worker-gone")
        .await
        .unwrap();
    store
        .mark_execution_running(orphan.id, Some("s1"))
        .await
        .unwrap();
    let approval = store
        .create_approval(
            org,
            orphan.id,
            serde_json::json!({ "capability": "issues.comment" }),
            None,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE executions SET created_at = now() - interval '1 hour', started_at = now() - interval '1 hour' WHERE id = $1")
        .bind(orphan.id)
        .execute(store.pool())
        .await
        .unwrap();

    // A live worker's execution must not be reaped.
    let healthy = make_execution("healthy-1").await;
    store
        .claim_execution(healthy.id, "worker-alive")
        .await
        .unwrap();
    store
        .mark_execution_running(healthy.id, Some("s2"))
        .await
        .unwrap();
    store
        .upsert_worker_heartbeat(
            org,
            uuid::Uuid::now_v7(),
            "worker-alive",
            &["mock".into()],
            "local",
            Some("test"),
        )
        .await
        .unwrap();

    let reaped = store
        .reap_orphaned_executions_for_org(org, 60)
        .await
        .unwrap();
    assert_eq!(reaped.len(), 1, "only the orphan should be reaped");
    assert_eq!(reaped[0].id, orphan.id);
    assert_eq!(reaped[0].status, "failed");
    assert_eq!(reaped[0].error.as_ref().unwrap()["code"], "worker_lost");

    assert_eq!(
        store.get_execution(org, healthy.id).await.unwrap().status,
        "running",
        "live worker execution must survive"
    );
    assert_eq!(
        store.get_approval(org, approval.id).await.unwrap().status,
        "expired"
    );
}

#[tokio::test]
async fn unstarted_executions_on_a_live_worker_are_ended() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;
    let (agent, version) = store
        .create_agent(org, Some(user), &definition("Unstarted Reaper Agent"))
        .await
        .unwrap();
    store
        .upsert_worker_heartbeat(
            org,
            uuid::Uuid::now_v7(),
            "worker-skewed",
            &["mock".into()],
            "local",
            Some("test"),
        )
        .await
        .unwrap();

    let claimed = |key: &'static str| {
        let store = &store;
        let agent = &agent;
        let version = &version;
        async move {
            let execution = store
                .create_execution(NewExecution {
                    org_id: org,
                    agent_id: agent.id,
                    agent_version_id: version.id,
                    agent_name: &agent.name,
                    harness_kind: HarnessKind::Mock,
                    trigger_source: TriggerSource::Manual,
                    placement: "auto",
                    trigger_id: None,
                    input: serde_json::json!({ "task": "x", "payload": {}, "provenance": { "source": "test", "trusted": true, "receivedAt": chrono::Utc::now() } }),
                    session_ref: None,
                    skills: &[],
                    timeout_seconds: 30,
                    requested_by: Some(user),
                    idempotency_key: Some(key),
                    team_run_id: None,
                    team_step_index: None,
                })
                .await
                .unwrap();
            store
                .claim_execution(execution.id, "worker-skewed")
                .await
                .unwrap();
            execution
        }
    };
    let age = |id: uuid::Uuid, interval: &'static str| {
        let store = &store;
        async move {
            sqlx::query(&format!(
                "UPDATE executions SET created_at = now() - interval '{interval}', \
                 started_at = now() - interval '{interval}', updated_at = now() - interval '{interval}' \
                 WHERE id = $1"
            ))
            .bind(id)
            .execute(store.pool())
            .await
            .unwrap();
            sqlx::query(&format!(
                "UPDATE execution_events SET created_at = now() - interval '{interval}' WHERE execution_id = $1"
            ))
            .bind(id)
            .execute(store.pool())
            .await
            .unwrap();
        }
    };

    // Claimed an hour ago, nothing since: the worker never got it.
    let silent = claimed("unstarted-silent").await;
    age(silent.id, "1 hour").await;

    // Claimed an hour ago but still reporting (a long checkout, say): leave it alone.
    let busy = claimed("unstarted-busy").await;
    age(busy.id, "1 hour").await;
    store
        .append_execution_event(
            org,
            busy.id,
            "status",
            LogLevel::Info,
            "checking out repository",
            &serde_json::json!({}),
        )
        .await
        .unwrap();

    // Cancel requested two minutes ago and ignored: end it as cancelled.
    let cancelled = claimed("unstarted-cancelled").await;
    store
        .request_execution_cancel(org, cancelled.id)
        .await
        .unwrap();
    age(cancelled.id, "2 minutes").await;

    // Started runs belong to the heartbeat reaper, not this one.
    let running = claimed("unstarted-running").await;
    store
        .mark_execution_running(running.id, Some("s"))
        .await
        .unwrap();
    age(running.id, "1 hour").await;

    let reaped = store
        .reap_unstarted_executions(Some(org), 600, 60)
        .await
        .unwrap();
    let mut ids: Vec<uuid::Uuid> = reaped.iter().map(|row| row.id).collect();
    ids.sort();
    let mut expected = vec![silent.id, cancelled.id];
    expected.sort();
    assert_eq!(ids, expected, "only the silent and the cancelled run end");

    let silent_now = store.get_execution(org, silent.id).await.unwrap();
    assert_eq!(silent_now.status, "failed");
    assert_eq!(silent_now.error.unwrap()["code"], "worker_did_not_start");
    assert_eq!(
        store.get_execution(org, cancelled.id).await.unwrap().status,
        "cancelled"
    );
    assert_eq!(
        store.get_execution(org, busy.id).await.unwrap().status,
        "dispatched"
    );
    assert_eq!(
        store.get_execution(org, running.id).await.unwrap().status,
        "running"
    );

    // A worker that finally reports in cannot revive an ended run.
    assert!(store
        .mark_execution_running(silent.id, Some("late"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn single_execution_mode_can_take_a_dispatched_execution() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;
    let (agent, version) = store
        .create_agent(org, Some(user), &definition("Single Execution Agent"))
        .await
        .unwrap();
    let execution = store
        .create_execution(NewExecution {
            org_id: org,
            agent_id: agent.id,
            agent_version_id: version.id,
            agent_name: &agent.name,
            harness_kind: HarnessKind::Mock,
            trigger_source: TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input: serde_json::json!({ "task": "x", "payload": {}, "provenance": { "source": "test", "trusted": true, "receivedAt": chrono::Utc::now() } }),
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(user),
            idempotency_key: Some("single-1"),
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();
    store
        .claim_execution(execution.id, "parent-worker")
        .await
        .unwrap();
    let taken = store
        .take_execution_for_worker(execution.id, "container-worker")
        .await
        .unwrap()
        .expect("dispatched execution must be takeable");
    assert_eq!(taken.worker_id.as_deref(), Some("container-worker"));
}

#[tokio::test]
async fn skills_profiles_and_agent_scoped_claims() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;

    let skill = store
        .create_skill(
            org,
            "issue-triage",
            "Issue triage",
            "Use when triaging issues",
            "---\nname: issue-triage\ndescription: triage\n---\n# Triage",
            &serde_json::json!({ "references/guide.md": "guide" }),
        )
        .await
        .unwrap();
    assert_eq!(skill.version, 1);
    let updated = store
        .update_skill(org, skill.id, Some("Triage v2"), None, None, None)
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
    assert_eq!(
        store
            .skills_by_slugs(org, &["issue-triage".into()])
            .await
            .unwrap()
            .len(),
        1
    );

    let (agent_a, version_a) = store
        .create_agent(org, Some(user), &definition("Scoped Agent A"))
        .await
        .unwrap();
    let (agent_b, version_b) = store
        .create_agent(org, Some(user), &definition("Scoped Agent B"))
        .await
        .unwrap();

    let input = serde_json::json!({
        "task": "x", "payload": {},
        "provenance": { "source": "test", "trusted": true, "receivedAt": chrono::Utc::now() }
    });
    let exec_a = store
        .create_execution(NewExecution {
            org_id: org,
            agent_id: agent_a.id,
            agent_version_id: version_a.id,
            agent_name: &agent_a.name,
            harness_kind: HarnessKind::Mock,
            trigger_source: TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input: input.clone(),
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(user),
            idempotency_key: Some("scoped-a"),
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();
    let _exec_b = store
        .create_execution(NewExecution {
            org_id: org,
            agent_id: agent_b.id,
            agent_version_id: version_b.id,
            agent_name: &agent_b.name,
            harness_kind: HarnessKind::Mock,
            trigger_source: TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input,
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(user),
            idempotency_key: Some("scoped-b"),
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();

    let claimed = store
        .claim_next_queued(
            org,
            "scoped-worker",
            &["mock".into()],
            "local",
            &[agent_a.id],
            None,
            None,
        )
        .await
        .unwrap()
        .expect("agent A execution should be claimable");
    assert_eq!(claimed.id, exec_a.id);

    let profile = store
        .create_worker_profile(
            org,
            "Scoped Pi",
            "pi",
            "docker",
            "process",
            "orbit-worker-pi",
            "0.3.0",
            "selected",
            &[agent_a.id],
            &["issue-triage".into()],
            Some("opencode-go"),
            Some("deepseek-v4-pro"),
            Some("orbit-pi-auth"),
            &serde_json::json!({}),
            &serde_json::json!([]),
            &serde_json::json!({}),
            false,
            false,
            false,
            false,
            &serde_json::json!({}),
        )
        .await
        .unwrap();
    assert_eq!(profile.agent_ids, vec![agent_a.id]);
    assert_eq!(profile.agent_scope, "selected");
    assert_eq!(profile.skill_slugs, vec!["issue-triage"]);
    assert!(
        !store
            .set_worker_profile_enabled(org, profile.id, false)
            .await
            .unwrap()
            .enabled
    );
    store.delete_worker_profile(org, profile.id).await.unwrap();
}

#[tokio::test]
async fn tool_policies_are_scoped_and_ordered() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, _user) = seed_org(&store).await;
    let (agent, _version) = store
        .create_agent(org, None, &definition("Tool policy target"))
        .await
        .expect("agent");

    let matcher = serde_json::json!({ "tool": "Bash", "commandRegex": "^rm " });
    let org_rule = store
        .create_tool_policy(
            org,
            None,
            "deny destructive shell",
            10,
            true,
            "deny",
            &matcher,
            Some("destructive command"),
            Some("deletes files"),
        )
        .await
        .expect("org rule");
    let agent_rule = store
        .create_tool_policy(
            org,
            Some(agent.id),
            "ask for writes",
            20,
            true,
            "require_approval",
            &serde_json::json!({ "effect": "write" }),
            None,
            None,
        )
        .await
        .expect("agent rule");

    // Org-level listing does not include agent rules.
    let org_rules = store.list_tool_policies(org, None, None).await.unwrap();
    assert_eq!(org_rules.len(), 1);
    assert_eq!(org_rules[0].id, org_rule.id);

    // Effective rules for the agent are ordered by priority.
    let rules = store
        .effective_tool_policy_rules(org, agent.id)
        .await
        .unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].name, "deny destructive shell");
    assert_eq!(rules[1].id, agent_rule.id);

    // A rule for a different agent is not visible.
    let (other, _) = store
        .create_agent(org, None, &definition("Other agent"))
        .await
        .expect("other agent");
    assert_eq!(
        store
            .effective_tool_policy_rules(org, other.id)
            .await
            .unwrap()
            .len(),
        1
    );

    store.delete_tool_policy(org, org_rule.id).await.unwrap();
    store.delete_tool_policy(org, agent_rule.id).await.unwrap();
    assert!(store
        .effective_tool_policy_rules(org, agent.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn custom_roles_round_trip() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;
    let role = store
        .create_custom_role(
            org,
            "Release manager",
            &["agents:read".to_string(), "executions:create".to_string()],
        )
        .await
        .expect("role");
    assert_eq!(
        store
            .custom_role_permissions(org, "Release manager")
            .await
            .unwrap(),
        Some(vec![
            "agents:read".to_string(),
            "executions:create".to_string()
        ])
    );
    assert!(store
        .custom_role_permissions(org, "missing")
        .await
        .unwrap()
        .is_none());

    // Users can be assigned the custom role by name.
    let updated = store
        .set_user_role_name(org, user, "Release manager")
        .await
        .unwrap();
    assert_eq!(updated.role, "Release manager");

    let updated = store
        .update_custom_role(
            org,
            role.id,
            "Release manager",
            &["agents:read".to_string(), "audit:read".to_string()],
        )
        .await
        .unwrap();
    assert_eq!(updated.permissions.len(), 2);

    store.delete_custom_role(org, role.id).await.unwrap();
    assert!(store.list_custom_roles(org).await.unwrap().is_empty());
}

#[tokio::test]
async fn env_mappings_and_outputs_round_trip() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, _user) = seed_org(&store).await;

    let mapping = store
        .create_env_mapping(
            org,
            "Shared tokens",
            "reusable",
            &serde_json::json!([
                { "envVar": "EXAMPLE_TOKEN", "connectionId": null, "secretKey": "example" }
            ]),
        )
        .await
        .expect("env mapping");
    assert_eq!(store.list_env_mappings(org).await.unwrap().len(), 1);
    assert_eq!(mapping.name, "Shared tokens");
    store.delete_env_mapping(org, mapping.id).await.unwrap();

    let output = store
        .create_output(
            org,
            "Slack release",
            "slack",
            None,
            None,
            &["succeeded".to_string(), "failed".to_string()],
            "secret://output/test",
            Some("release finished"),
        )
        .await
        .expect("output");
    assert_eq!(store.list_outputs(org).await.unwrap().len(), 1);
    assert_eq!(
        store.get_output(org, output.id).await.unwrap().kind,
        "slack"
    );
    store.delete_output(org, output.id).await.unwrap();
}

#[tokio::test]
async fn access_teams_and_ownership_defaults() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;

    let teams = store.list_access_teams(org).await.unwrap();
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].slug, "default");

    let team = store
        .create_access_team(org, "Payments", "payments", Some(user))
        .await
        .expect("team");
    let member = store
        .upsert_team_member(team.id, user, "lead")
        .await
        .expect("member");
    assert_eq!(member.role, "lead");
    assert_eq!(store.list_team_members(team.id).await.unwrap().len(), 1);

    let updated = store
        .update_access_team(org, team.id, "Payments team", "payments-team")
        .await
        .expect("update");
    assert_eq!(updated.slug, "payments-team");

    let agent = store
        .create_agent(org, Some(user), &definition("Owned agent"))
        .await
        .expect("agent")
        .0;
    let loaded = store.get_agent(org, agent.id).await.unwrap();
    assert_eq!(loaded.visibility, "org");
    assert!(loaded.owner_team_id.is_some());

    store.remove_team_member(team.id, user).await.unwrap();
    store.delete_access_team(org, team.id).await.unwrap();
}

#[tokio::test]
async fn lists_honour_strict_team_isolation() {
    use orbit_store::access::ResourceAccess;

    let Some(store) = test_store().await else {
        return;
    };
    let (org, owner) = seed_org(&store).await;
    let viewer = store
        .create_user(org, "operator@test.local", "Operator", UserRole::Operator)
        .await
        .expect("user");
    let ours = store
        .create_access_team(org, "Ours", "ours", Some(owner))
        .await
        .expect("team");
    let theirs = store
        .create_access_team(org, "Theirs", "theirs", Some(owner))
        .await
        .expect("team");
    store
        .upsert_team_member(ours.id, viewer.id, "member")
        .await
        .unwrap();

    let agent = |name: &'static str, visibility: &'static str, team: Option<uuid::Uuid>| {
        let store = &store;
        async move {
            let created = store
                .create_agent(org, Some(owner), &definition(name))
                .await
                .expect("agent")
                .0;
            store
                .set_resource_visibility(org, "agents", created.id, visibility, team)
                .await
                .expect("visibility");
        }
    };
    agent("Org, other team", "org", Some(theirs.id)).await;
    agent("Org, our team", "org", Some(ours.id)).await;
    agent("Org, no team", "org", None).await;
    agent("Private, other team", "private", Some(theirs.id)).await;

    let names = |access: ResourceAccess| {
        let store = &store;
        async move {
            let (agents, _, total) = store
                .list_agents(org, 50, None, None, None, Some(&access))
                .await
                .expect("list");
            let mut names: Vec<String> = agents.into_iter().map(|agent| agent.agent.name).collect();
            names.sort();
            assert_eq!(total as usize, names.len(), "count matches the page");
            names
        }
    };
    let member = ResourceAccess::new(viewer.id, false, vec![ours.id]);

    assert_eq!(
        names(member.clone()).await,
        ["Org, no team", "Org, other team", "Org, our team"],
        "without isolation, organization visibility is enough"
    );
    assert_eq!(
        names(member.with_team_isolation(true)).await,
        ["Org, no team", "Org, our team"],
        "with isolation, another team's resources stay hidden"
    );
    assert_eq!(
        names(ResourceAccess::new(owner, true, vec![]).with_team_isolation(true))
            .await
            .len(),
        4,
        "admins see everything"
    );
}

#[tokio::test]
async fn team_worker_cannot_claim_another_teams_execution() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;
    let team_a = store
        .create_access_team(org, "Team A", "team-a", Some(user))
        .await
        .unwrap();
    let team_b = store
        .create_access_team(org, "Team B", "team-b", Some(user))
        .await
        .unwrap();
    let (agent_a, version_a) = store
        .create_agent(org, Some(user), &definition("Team A Agent"))
        .await
        .unwrap();
    let (agent_b, version_b) = store
        .create_agent(org, Some(user), &definition("Team B Agent"))
        .await
        .unwrap();
    store
        .set_resource_visibility(org, "agents", agent_a.id, "team", Some(team_a.id))
        .await
        .unwrap();
    store
        .set_resource_visibility(org, "agents", agent_b.id, "team", Some(team_b.id))
        .await
        .unwrap();

    let stored_a = store.get_agent(org, agent_a.id).await.unwrap();
    let stored_b = store.get_agent(org, agent_b.id).await.unwrap();
    assert_eq!(stored_a.visibility, "team");
    assert_eq!(stored_a.owner_team_id, Some(team_a.id));
    assert_eq!(stored_b.visibility, "team");
    assert_eq!(stored_b.owner_team_id, Some(team_b.id));
    assert_ne!(team_a.id, team_b.id);

    let input = serde_json::json!({
        "task": "x", "payload": {},
        "provenance": { "source": "test", "trusted": true, "receivedAt": chrono::Utc::now() }
    });
    let exec_b = store
        .create_execution(NewExecution {
            org_id: org,
            agent_id: agent_b.id,
            agent_version_id: version_b.id,
            agent_name: &agent_b.name,
            harness_kind: HarnessKind::Mock,
            trigger_source: TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input: input.clone(),
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(user),
            idempotency_key: Some("team-b-claim"),
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();
    let exec_a = store
        .create_execution(NewExecution {
            org_id: org,
            agent_id: agent_a.id,
            agent_version_id: version_a.id,
            agent_name: &agent_a.name,
            harness_kind: HarnessKind::Mock,
            trigger_source: TriggerSource::Manual,
            placement: "auto",
            trigger_id: None,
            input,
            session_ref: None,
            skills: &[],
            timeout_seconds: 30,
            requested_by: Some(user),
            idempotency_key: Some("team-a-claim"),
            team_run_id: None,
            team_step_index: None,
        })
        .await
        .unwrap();

    assert_eq!(exec_a.agent_id, agent_a.id);
    assert_eq!(exec_b.agent_id, agent_b.id);
    let after_a = store.get_agent(org, agent_a.id).await.unwrap();
    let after_b = store.get_agent(org, agent_b.id).await.unwrap();
    assert_eq!(after_a.visibility, "team");
    assert_eq!(after_b.visibility, "team");

    // A random team must not claim anything.
    let none = store
        .claim_next_queued(
            org,
            "random-worker",
            &["mock".into()],
            "local",
            &[],
            Some(uuid::Uuid::now_v7()),
            None,
        )
        .await
        .unwrap();
    assert!(none.is_none(), "claim ignored the team predicate");

    // B was queued first but is invisible to Team A.
    let claimed = store
        .claim_next_queued(
            org,
            "team-a-worker",
            &["mock".into()],
            "local",
            &[],
            Some(team_a.id),
            None,
        )
        .await
        .unwrap()
        .expect("Team A execution should be claimable");
    assert_eq!(claimed.id, exec_a.id);

    let next = store
        .claim_next_queued(
            org,
            "team-a-worker",
            &["mock".into()],
            "local",
            &[],
            Some(team_a.id),
            None,
        )
        .await
        .unwrap();
    assert!(next.is_none(), "Team A worker claimed Team B execution");

    // The Team B execution is still queued.
    let still_queued = store.get_execution(org, exec_b.id).await.unwrap();
    assert_eq!(still_queued.status, "queued");
}

#[tokio::test]
async fn agent_spec_triggers_are_reconciled() {
    let Some(store) = test_store().await else {
        return;
    };
    let (org, user) = seed_org(&store).await;
    let mut first = definition("Trigger Reconciler");
    first.spec.triggers = vec![TriggerSpec::Schedule {
        cron: "*/15 * * * *".into(),
        timezone: None,
        task: Some("first task".into()),
    }];
    let (agent, _) = store.create_agent(org, Some(user), &first).await.unwrap();
    let triggers = store
        .list_triggers(org, Some(agent.id), None)
        .await
        .unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].config["cron"], "*/15 * * * *");

    let mut second = definition("Trigger Reconciler");
    second.spec.triggers = vec![TriggerSpec::Schedule {
        cron: "*/5 * * * *".into(),
        timezone: None,
        task: Some("five minute task".into()),
    }];
    store
        .add_agent_version(org, agent.id, Some(user), &second)
        .await
        .unwrap();
    let triggers = store
        .list_triggers(org, Some(agent.id), None)
        .await
        .unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0].config["cron"], "*/5 * * * *");
    assert_eq!(triggers[0].config["task"], "five minute task");

    let mut third = definition("Trigger Reconciler");
    third.spec.triggers = vec![];
    store
        .add_agent_version(org, agent.id, Some(user), &third)
        .await
        .unwrap();
    assert!(store
        .list_triggers(org, Some(agent.id), None)
        .await
        .unwrap()
        .is_empty());
}
