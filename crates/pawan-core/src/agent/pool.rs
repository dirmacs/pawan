use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Instant;

use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

/// Status of a pooled task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

/// Pool event stream for progress reporting.
#[derive(Debug, Clone)]
pub enum AgentPoolEvent {
    Progress { completed: usize, total: usize },
}

/// Input task for the pool.
#[derive(Debug, Clone)]
pub struct PoolTask {
    pub id: String,
    pub agent_type: String,
    pub assignment: String,
    pub context: Option<String>,
}

/// Result record for a pooled task.
#[derive(Debug, Clone)]
pub struct PoolResult {
    pub id: String,
    pub status: TaskStatus, // Completed, Failed, Cancelled, TimedOut
    pub output: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

#[async_trait::async_trait]
pub trait PoolExecutor: Send + Sync + 'static {
    async fn execute_task(&self, task: PoolTask, cancel: CancellationToken) -> PoolResult;
}

/// Run multiple tasks concurrently with a concurrency limit.
pub struct AgentPool {
    pub max_concurrent: usize, // default: number of CPU cores
    pub tasks: Vec<PoolTask>,
    pub results: Vec<PoolResult>,
    stop_on_error: bool,
    cancel: CancellationToken,
    progress_tx: Option<mpsc::UnboundedSender<AgentPoolEvent>>,
    executor: Arc<dyn PoolExecutor>,
}

impl AgentPool {
    pub fn new(executor: Arc<dyn PoolExecutor>) -> Self {
        let default_concurrency = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        Self {
            max_concurrent: default_concurrency,
            tasks: Vec::new(),
            results: Vec::new(),
            stop_on_error: false,
            cancel: CancellationToken::new(),
            progress_tx: None,
            executor,
        }
    }

    pub fn with_max_concurrent(mut self, max_concurrent: usize) -> Self {
        self.max_concurrent = max_concurrent.max(1);
        self
    }

    pub fn with_stop_on_error(mut self, stop_on_error: bool) -> Self {
        self.stop_on_error = stop_on_error;
        self
    }

    pub fn with_progress_sender(mut self, tx: mpsc::UnboundedSender<AgentPoolEvent>) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Execute all tasks, returning results in the original order.
    pub async fn execute(&mut self) -> Vec<PoolResult> {
        self.results.clear();

        if self.tasks.is_empty() {
            return Vec::new();
        }

        let total = self.tasks.len();
        let semaphore = Arc::new(Semaphore::new(self.max_concurrent.max(1)));
        let completed = Arc::new(AtomicUsize::new(0));
        let has_failed = Arc::new(AtomicBool::new(false));

        let mut handles = Vec::with_capacity(total);

        for (idx, task) in self.tasks.clone().into_iter().enumerate() {
            let sem = semaphore.clone();
            let executor = self.executor.clone();
            let cancel = self.cancel.clone();
            let completed_ctr = completed.clone();
            let has_failed_flag = has_failed.clone();
            let stop_on_error = self.stop_on_error;
            let progress_tx = self.progress_tx.clone();

            let handle = tokio::task::spawn(async move {
                if cancel.is_cancelled() {
                    return (
                        idx,
                        PoolResult {
                            id: task.id.clone(),
                            status: TaskStatus::Cancelled,
                            output: None,
                            error: Some("cancelled".to_string()),
                            duration_ms: 0,
                        },
                    );
                }

                // Acquire concurrency permit (cancellation-aware).
                let permit = tokio::select! {
                    _ = cancel.cancelled() => {
                        return (idx, PoolResult {
                            id: task.id.clone(),
                            status: TaskStatus::Cancelled,
                            output: None,
                            error: Some("cancelled".to_string()),
                            duration_ms: 0,
                        });
                    }
                    p = sem.acquire() => p,
                };

                let _permit = match permit {
                    Ok(p) => p,
                    Err(_) => {
                        return (
                            idx,
                            PoolResult {
                                id: task.id.clone(),
                                status: TaskStatus::Failed,
                                output: None,
                                error: Some("semaphore closed".to_string()),
                                duration_ms: 0,
                            },
                        );
                    }
                };

                if cancel.is_cancelled() {
                    return (
                        idx,
                        PoolResult {
                            id: task.id.clone(),
                            status: TaskStatus::Cancelled,
                            output: None,
                            error: Some("cancelled".to_string()),
                            duration_ms: 0,
                        },
                    );
                }

                let started = Instant::now();
                let mut result = executor.execute_task(task, cancel.clone()).await;
                result.duration_ms = started.elapsed().as_millis() as u64;

                if stop_on_error && result.status == TaskStatus::Failed {
                    has_failed_flag.store(true, Ordering::SeqCst);
                    cancel.cancel();
                }

                let done = completed_ctr.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some(tx) = progress_tx {
                    let _ = tx.send(AgentPoolEvent::Progress {
                        completed: done,
                        total,
                    });
                }

                if stop_on_error
                    && has_failed_flag.load(Ordering::SeqCst)
                    && result.status != TaskStatus::Completed
                    && result.status != TaskStatus::Failed
                {
                    result.status = TaskStatus::Cancelled;
                    result.output = None;
                    if result.error.is_none() {
                        result.error = Some("cancelled".to_string());
                    }
                }

                (idx, result)
            });

            handles.push(handle);
        }

        let mut out: Vec<Option<PoolResult>> = vec![None; total];
        for h in handles {
            match h.await {
                Ok((idx, r)) => out[idx] = Some(r),
                Err(join_err) => {
                    let r = PoolResult {
                        id: "<join>".to_string(),
                        status: TaskStatus::Failed,
                        output: None,
                        error: Some(format!("join error: {join_err}")),
                        duration_ms: 0,
                    };
                    if let Some(slot) = out.iter_mut().find(|s| s.is_none()) {
                        *slot = Some(r);
                    }
                }
            }
        }

        let results: Vec<PoolResult> = out
            .into_iter()
            .map(|r| {
                r.unwrap_or(PoolResult {
                    id: "<missing>".to_string(),
                    status: TaskStatus::Cancelled,
                    output: None,
                    error: Some("missing result".to_string()),
                    duration_ms: 0,
                })
            })
            .collect();

        self.results = results.clone();
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    struct TestExecutor;

    #[async_trait::async_trait]
    impl PoolExecutor for TestExecutor {
        async fn execute_task(&self, task: PoolTask, cancel: CancellationToken) -> PoolResult {
            tokio::select! {
                _ = cancel.cancelled() => {
                    PoolResult {
                        id: task.id.clone(),
                        status: TaskStatus::Cancelled,
                        output: None,
                        error: Some("cancelled".to_string()),
                        duration_ms: 0,
                    }
                }
                _ = sleep(Duration::from_millis(25)) => {
                    if task.assignment.contains("fail") {
                        PoolResult {
                            id: task.id.clone(),
                            status: TaskStatus::Failed,
                            output: None,
                            error: Some("boom".to_string()),
                            duration_ms: 0,
                        }
                    } else {
                        PoolResult {
                            id: task.id.clone(),
                            status: TaskStatus::Completed,
                            output: Some(format!("ok:{}", task.id)),
                            error: None,
                            duration_ms: 0,
                        }
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn pool_empty_returns_empty_vec() {
        let exec = Arc::new(TestExecutor);
        let mut pool = AgentPool::new(exec);
        let results = pool.execute().await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn pool_three_tasks_all_complete() {
        let exec = Arc::new(TestExecutor);
        let mut pool = AgentPool::new(exec).with_max_concurrent(2);
        pool.tasks = vec![
            PoolTask {
                id: "a".into(),
                agent_type: "t".into(),
                assignment: "ok".into(),
                context: None,
            },
            PoolTask {
                id: "b".into(),
                agent_type: "t".into(),
                assignment: "ok".into(),
                context: None,
            },
            PoolTask {
                id: "c".into(),
                agent_type: "t".into(),
                assignment: "ok".into(),
                context: None,
            },
        ];

        let results = pool.execute().await;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].id, "a");
        assert_eq!(results[1].id, "b");
        assert_eq!(results[2].id, "c");
        assert!(results.iter().all(|r| r.status == TaskStatus::Completed));
    }

    #[tokio::test]
    async fn pool_stop_on_error_cancels_remaining() {
        let exec = Arc::new(TestExecutor);
        let mut pool = AgentPool::new(exec)
            .with_max_concurrent(3)
            .with_stop_on_error(true);

        pool.tasks = vec![
            PoolTask {
                id: "ok1".into(),
                agent_type: "t".into(),
                assignment: "ok".into(),
                context: None,
            },
            PoolTask {
                id: "bad".into(),
                agent_type: "t".into(),
                assignment: "fail".into(),
                context: None,
            },
            PoolTask {
                id: "ok2".into(),
                agent_type: "t".into(),
                assignment: "ok".into(),
                context: None,
            },
        ];

        let results = pool.execute().await;
        assert_eq!(results.len(), 3);
        assert!(results
            .iter()
            .any(|r| r.id == "bad" && r.status == TaskStatus::Failed));
        assert!(results
            .iter()
            .filter(|r| r.id != "bad")
            .all(|r| r.status == TaskStatus::Completed || r.status == TaskStatus::Cancelled));
    }
}
