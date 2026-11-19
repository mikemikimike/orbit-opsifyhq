//! Close-the-loop for Agent Team queue items.
//!
//! When a TeamRun terminates, mirror its outcome back to the source issue through the
//! configured connector: remove the claim label, add a done/failed label and (on success)
//! post a comment with any PR/MR links found in the execution output.

use serde_json::Value;

use orbit_store::team_queue::TeamQueueItem;
use orbit_store::TeamRun;

use crate::handlers::teams::{parse_definition, TeamDefinition};
use crate::state::AppState;
use crate::work_sources::{self, WorkSourceDefinition};

struct WorkSourceConnector {
    source: WorkSourceDefinition,
    base_url: String,
    token: Option<String>,
    client: reqwest::Client,
}

pub(crate) async fn close_queue_loop(
    state: &AppState,
    run: &TeamRun,
    status: &str,
    output: Option<&Value>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    let Some(item) = state.store.get_team_queue_item_by_run(run.id).await? else {
        return Ok(());
    };
    let definition = definition_for_run(state, run).await;
    let source = definition
        .as_ref()
        .and_then(|definition| definition.work_source.clone());
    let connector = match source {
        Some(source) => connector_for_work_source(state, run.org_id, &source).await?,
        None => None,
    };

    match status {
        "succeeded" => {
            state
                .store
                .set_team_queue_status_by_run(run.id, "succeeded", None)
                .await?;
            if let Some(connector) = &connector {
                if let Err(error) = apply_outcome(connector, &item, "done", output).await {
                    tracing::warn!(%error, queue_item = %item.id, "failed to apply done outcome to issue");
                }
            }
        }
        "failed" => {
            let retry = item.max_attempts > item.attempts + 1;
            if retry {
                state
                    .store
                    .retry_team_queue_item(run.org_id, item.id, error)
                    .await?;
                if let Some(connector) = &connector {
                    if let Err(error) = apply_outcome(connector, &item, "retry", None).await {
                        tracing::warn!(%error, queue_item = %item.id, "failed to release claim for retry");
                    }
                }
            } else {
                state
                    .store
                    .set_team_queue_status_by_run(run.id, "failed", error)
                    .await?;
                if let Some(connector) = &connector {
                    if let Err(error) = apply_outcome(connector, &item, "failed", None).await {
                        tracing::warn!(%error, queue_item = %item.id, "failed to apply failed outcome to issue");
                    }
                }
            }
        }
        "cancelled" => {
            state
                .store
                .set_team_queue_status_by_run(run.id, "cancelled", error)
                .await?;
            if let Some(connector) = &connector {
                if let Err(error) = apply_outcome(connector, &item, "cancelled", None).await {
                    tracing::warn!(%error, queue_item = %item.id, "failed to release claim on cancel");
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn definition_for_run(state: &AppState, run: &TeamRun) -> Option<TeamDefinition> {
    if let Some(value) = run.input.get("definition") {
        if let Ok(definition) = serde_json::from_value::<TeamDefinition>(value.clone()) {
            return Some(definition);
        }
    }
    let team = state
        .store
        .get_agent_team(run.org_id, run.team_id)
        .await
        .ok()?;
    parse_definition(&team).ok()
}

async fn connector_for_work_source(
    state: &AppState,
    org_id: uuid::Uuid,
    source: &WorkSourceDefinition,
) -> anyhow::Result<Option<WorkSourceConnector>> {
    if !source.enabled {
        return Ok(None);
    }
    let Some(connection_id) = source.connection_id else {
        return Ok(None);
    };
    let connection = state.store.get_connection(org_id, connection_id).await?;
    let token = match &connection.credential_ref {
        Some(reference) => state
            .store
            .get_secret_value(reference)
            .await?
            .map(|secret| secret.expose().to_string()),
        None => None,
    };
    let base_url = match source.provider.as_str() {
        "github" => connection
            .config
            .get("baseUrl")
            .and_then(Value::as_str)
            .unwrap_or("https://api.github.com")
            .trim_end_matches('/')
            .to_string(),
        "gitlab" => connection
            .config
            .get("baseUrl")
            .and_then(Value::as_str)
            .unwrap_or("https://gitlab.com/api/v4")
            .trim_end_matches('/')
            .to_string(),
        other => anyhow::bail!("unsupported work source provider '{other}'"),
    };
    orbit_security::check_outbound_url(&base_url, &orbit_security::OutboundPolicy::from_env())
        .await
        .map_err(|error| anyhow::anyhow!("work source baseUrl refused: {error}"))?;
    Ok(Some(WorkSourceConnector {
        source: source.clone(),
        base_url,
        token,
        client: orbit_security::guarded_client(std::time::Duration::from_secs(20)),
    }))
}

async fn apply_outcome(
    connector: &WorkSourceConnector,
    item: &TeamQueueItem,
    outcome: &str,
    output: Option<&Value>,
) -> anyhow::Result<()> {
    let number = parse_number(&item.external_key)?;
    match connector.source.provider.as_str() {
        "github" => {
            let repository = item
                .repository
                .clone()
                .or_else(|| connector.source.repository.clone())
                .ok_or_else(|| anyhow::anyhow!("queue item has no repository"))?;
            let current = current_labels(connector, &repository, number, None).await?;
            let labels = github_outcome_labels(&connector.source, &current, outcome);
            work_sources::update_github_labels(
                &connector.client,
                &connector.base_url,
                connector.token.as_deref(),
                &repository,
                number,
                &labels,
            )
            .await?;
            if outcome == "done" {
                let body = success_comment(output);
                work_sources::comment_github(
                    &connector.client,
                    &connector.base_url,
                    connector.token.as_deref(),
                    &repository,
                    number,
                    &body,
                )
                .await?;
            }
        }
        "gitlab" => {
            let project = item
                .repository
                .clone()
                .or_else(|| connector.source.project.clone())
                .ok_or_else(|| anyhow::anyhow!("queue item has no GitLab project"))?;
            let current = current_labels(connector, &project, number, Some(&project)).await?;
            let (add, remove) = gitlab_outcome_labels(&connector.source, &current, outcome);
            work_sources::update_gitlab_labels(
                &connector.client,
                &connector.base_url,
                connector.token.as_deref(),
                &project,
                number,
                &add,
                &remove,
            )
            .await?;
            if outcome == "done" {
                let body = success_comment(output);
                work_sources::comment_gitlab(
                    &connector.client,
                    &connector.base_url,
                    connector.token.as_deref(),
                    &project,
                    number,
                    &body,
                )
                .await?;
            }
        }
        other => anyhow::bail!("unsupported work source provider '{other}'"),
    }
    Ok(())
}

async fn current_labels(
    connector: &WorkSourceConnector,
    target: &str,
    number: u64,
    project: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    if connector.source.provider == "github" {
        Ok(work_sources::get_github_issue(
            &connector.client,
            &connector.base_url,
            connector.token.as_deref(),
            target,
            number,
        )
        .await?
        .labels)
    } else {
        let project =
            project.ok_or_else(|| anyhow::anyhow!("GitLab outcome requires a project"))?;
        Ok(work_sources::get_gitlab_issue(
            &connector.client,
            &connector.base_url,
            connector.token.as_deref(),
            project,
            number,
        )
        .await?
        .labels)
    }
}

fn github_outcome_labels(
    source: &WorkSourceDefinition,
    current: &[String],
    outcome: &str,
) -> Vec<String> {
    let mut labels: Vec<String> = current
        .iter()
        .filter(|label| *label != &source.claim_label && !source.labels.contains(label))
        .cloned()
        .collect();
    match outcome {
        "done" => labels.push(source.done_label.clone()),
        "failed" | "cancelled" => labels.push(source.failed_label.clone()),
        "retry" => labels.extend(source.labels.clone()),
        _ => {}
    }
    labels.sort();
    labels.dedup();
    labels
}

fn gitlab_outcome_labels(
    source: &WorkSourceDefinition,
    current: &[String],
    outcome: &str,
) -> (Vec<String>, Vec<String>) {
    let mut add = Vec::new();
    let mut remove = vec![source.claim_label.clone()];
    match outcome {
        "done" => add.push(source.done_label.clone()),
        "failed" | "cancelled" => add.push(source.failed_label.clone()),
        "retry" => {
            remove.extend(source.labels.clone());
            add.extend(source.labels.clone());
        }
        _ => {}
    }
    let _ = current;
    (add, remove)
}

fn parse_number(external_key: &str) -> anyhow::Result<u64> {
    external_key
        .rsplit_once('#')
        .and_then(|(_, number)| number.parse::<u64>().ok())
        .ok_or_else(|| anyhow::anyhow!("external key has no numeric issue id"))
}

fn success_comment(output: Option<&Value>) -> String {
    let links = collect_links(output);
    let mut body = "ORBIT agent team completed this issue.".to_string();
    if !links.is_empty() {
        body.push_str("\n\nLinked changes:\n");
        for link in links {
            body.push_str(&format!("- {link}\n"));
        }
    }
    body
}

fn collect_links(value: Option<&Value>) -> Vec<String> {
    fn walk(value: &Value, links: &mut Vec<String>) {
        match value {
            Value::String(value) => {
                if (value.contains("pull/") || value.contains("merge_requests"))
                    && (value.starts_with("http://") || value.starts_with("https://"))
                {
                    links.push(value.clone());
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, links);
                }
            }
            Value::Object(map) => {
                for (_key, value) in map {
                    walk(value, links);
                }
            }
            _ => {}
        }
    }
    let mut links = Vec::new();
    if let Some(value) = value {
        walk(value, &mut links);
    }
    links.sort();
    links.dedup();
    links.truncate(10);
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_label_transitions() {
        let source = WorkSourceDefinition {
            labels: vec!["agent-ready".into()],
            ..Default::default()
        };
        let current = vec!["agent-ready".into(), "agent-in-progress".into()];
        let done = github_outcome_labels(&source, &current, "done");
        assert!(done.contains(&"agent-done".into()));
        assert!(!done.contains(&"agent-ready".into()));
        assert!(!done.contains(&"agent-in-progress".into()));

        let failed = github_outcome_labels(&source, &current, "failed");
        assert!(failed.contains(&"agent-failed".into()));
    }

    #[test]
    fn extracts_links_from_output() {
        let output = serde_json::json!({
            "pullRequests": [{ "htmlUrl": "https://github.com/acme/orbit/pull/12" }]
        });
        assert_eq!(
            collect_links(Some(&output)),
            vec!["https://github.com/acme/orbit/pull/12".to_string()]
        );
    }
}
