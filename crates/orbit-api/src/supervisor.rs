//! Supervision for long-running background loops: cancellation on shutdown and restart
//! (with backoff and a logged error) when a loop panics.

use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::state::AppState;

/// Delay before a panicked loop is restarted.
const RESTART_BACKOFF: Duration = Duration::from_secs(5);

/// Run `make(state)` as a supervised background task named `name`.
///
/// * Stops (aborting the in-flight future) as soon as `state.shutdown` is cancelled.
/// * If the task panics, logs the panic with the loop name and restarts it after a backoff.
/// * If the task returns normally it is not restarted (the loops here never return).
pub fn supervise<F, Fut>(name: &'static str, state: &AppState, make: F) -> JoinHandle<()>
where
    F: Fn(AppState) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let state = state.clone();
    let token = state.shutdown.clone();
    tokio::spawn(supervise_with(name, token, RESTART_BACKOFF, move || {
        make(state.clone())
    }))
}

/// State-independent core of [`supervise`], separated so it can be unit tested.
pub async fn supervise_with<F, Fut>(
    name: &'static str,
    token: CancellationToken,
    backoff: Duration,
    make: F,
) where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    loop {
        let mut task = tokio::spawn(make());
        tokio::select! {
            _ = token.cancelled() => {
                task.abort();
                let _ = task.await;
                tracing::debug!(task = name, "background loop stopped");
                return;
            }
            result = &mut task => match result {
                Ok(()) => {
                    tracing::warn!(task = name, "background loop exited; not restarting");
                    return;
                }
                Err(error) if error.is_panic() => {
                    tracing::error!(
                        task = name,
                        backoff_secs = backoff.as_secs_f32(),
                        "background loop panicked; restarting"
                    );
                    tokio::select! {
                        _ = token.cancelled() => return,
                        _ = tokio::time::sleep(backoff) => {}
                    }
                }
                Err(_) => return,
            },
        }
    }
}

#[cfg(test)]
mod supervisor_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn restarts_a_panicking_loop_and_stops_on_cancel() {
        let runs = Arc::new(AtomicUsize::new(0));
        let token = CancellationToken::new();
        let counter = runs.clone();
        let supervisor = tokio::spawn(supervise_with(
            "test",
            token.clone(),
            Duration::from_millis(10),
            move || {
                let counter = counter.clone();
                async move {
                    if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                        panic!("boom");
                    }
                    std::future::pending::<()>().await;
                }
            },
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            while runs.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("loop restarted after panic");
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), supervisor)
            .await
            .expect("supervisor stops on cancel")
            .unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_stuck_loop() {
        let token = CancellationToken::new();
        let supervisor = tokio::spawn(supervise_with(
            "stuck",
            token.clone(),
            Duration::from_millis(10),
            std::future::pending::<()>,
        ));
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), supervisor)
            .await
            .expect("stops promptly")
            .unwrap();
    }
}
