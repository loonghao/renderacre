use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::models::{
    DashboardSnapshot, FarmLogEntry, FarmStats, Job, JobId, JobState, JobSubmit, LogLevel,
    LogSource, Task, TaskArtifact, TaskComplete, TaskId, TaskLease, TaskLeaseInfo,
    TaskLeaseRenewal, TaskStarted, TaskState, TaskSubmit, WorkerCapacity, WorkerId, WorkerInfo,
    WorkerLogBatch, WorkerRegister, WorkerState,
};
use crate::openjd::openjd_to_tasks;

#[derive(Debug, Error)]
pub enum FarmError {
    #[error("job was not found: {0}")]
    JobNotFound(JobId),
    #[error("task was not found: {0}")]
    TaskNotFound(TaskId),
    #[error("worker was not found: {0}")]
    WorkerNotFound(WorkerId),
    #[error("artifact was not found for task {task_id}: {artifact_index}")]
    ArtifactNotFound {
        task_id: TaskId,
        artifact_index: usize,
    },
    #[error("invalid submission: {0}")]
    InvalidSubmission(String),
    #[error("invalid state transition: {0}")]
    InvalidState(String),
    #[error("lease is invalid or expired")]
    InvalidLease,
    #[error("scheduler lock was poisoned")]
    LockPoisoned,
    #[error("scheduler storage failed: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryScheduler {
    inner: Arc<Mutex<SchedulerState>>,
    config: SchedulerConfig,
}

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub lease_ttl_seconds: i64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            lease_ttl_seconds: 120,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SchedulerState {
    jobs: HashMap<JobId, Job>,
    workers: HashMap<WorkerId, WorkerInfo>,
    logs: Vec<FarmLogEntry>,
}

const MAX_LOG_ENTRIES: usize = 500;

impl InMemoryScheduler {
    pub fn with_config(config: SchedulerConfig) -> Self {
        Self {
            inner: Arc::default(),
            config,
        }
    }

    fn from_state(state: SchedulerState, config: SchedulerConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(state)),
            config,
        }
    }

    fn state_snapshot(&self) -> Result<SchedulerState, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        Ok(state.clone())
    }

    fn replace_state(&self, replacement: SchedulerState) -> Result<(), FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        *state = replacement;
        Ok(())
    }

    pub fn submit_job(&self, submission: JobSubmit) -> Result<Job, FarmError> {
        let mut tasks = submission.tasks;
        if tasks.is_empty() {
            if let Some(openjd) = &submission.openjd {
                tasks = openjd_to_tasks(openjd)?;
            }
        }

        if tasks.is_empty() {
            return Err(FarmError::InvalidSubmission(
                "job must include direct tasks or an OpenJD template with steps".to_string(),
            ));
        }

        let now = Utc::now();
        let job_id = Uuid::new_v4();
        let task_ids_by_name = tasks
            .iter()
            .map(|task| (task.name.clone(), Uuid::new_v4()))
            .collect::<HashMap<_, _>>();

        let mut names = HashSet::new();
        for task in &tasks {
            if !names.insert(task.name.clone()) {
                return Err(FarmError::InvalidSubmission(format!(
                    "duplicate task name '{}'",
                    task.name
                )));
            }
        }

        let job_tasks = tasks
            .into_iter()
            .map(|task| {
                task_from_submit(job_id, &task_ids_by_name, task, submission.max_retries, now)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let job = Job {
            id: job_id,
            name: submission.name,
            state: JobState::Queued,
            priority: submission.priority,
            created_at: now,
            updated_at: now,
            tasks: job_tasks,
            openjd: submission.openjd,
        };

        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        state.jobs.insert(job.id, job.clone());
        push_log(
            &mut state,
            NewLog {
                level: LogLevel::Info,
                source: LogSource::Controller,
                message: format!("job submitted: {}", job.name),
                job_id: Some(job.id),
                task_id: None,
                worker_id: None,
                stream: None,
            },
        );
        Ok(job)
    }

    pub fn get_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        state
            .jobs
            .get(&job_id)
            .cloned()
            .ok_or(FarmError::JobNotFound(job_id))
    }

    pub fn list_jobs(&self) -> Result<Vec<Job>, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let mut jobs = state.jobs.values().cloned().collect::<Vec<_>>();
        jobs.sort_by_key(|job| std::cmp::Reverse((job.priority, job.created_at)));
        Ok(jobs)
    }

    pub fn list_workers(&self) -> Result<Vec<WorkerInfo>, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let mut workers = state.workers.values().cloned().collect::<Vec<_>>();
        workers.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(workers)
    }

    pub fn dashboard_snapshot(&self) -> Result<DashboardSnapshot, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let mut jobs = state.jobs.values().cloned().collect::<Vec<_>>();
        jobs.sort_by_key(|job| std::cmp::Reverse((job.priority, job.created_at)));
        let mut workers = state.workers.values().cloned().collect::<Vec<_>>();
        workers.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(DashboardSnapshot {
            stats: compute_stats(&state),
            jobs,
            workers,
            logs: state.logs.clone(),
        })
    }

    pub fn list_logs(&self) -> Result<Vec<FarmLogEntry>, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        Ok(state.logs.clone())
    }

    pub fn list_worker_logs(&self, worker_id: WorkerId) -> Result<Vec<FarmLogEntry>, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        if !state.workers.contains_key(&worker_id) {
            return Err(FarmError::WorkerNotFound(worker_id));
        }
        Ok(state
            .logs
            .iter()
            .filter(|entry| entry.worker_id == Some(worker_id))
            .cloned()
            .collect())
    }

    pub fn register_worker(&self, registration: WorkerRegister) -> Result<WorkerInfo, FarmError> {
        let now = Utc::now();
        let worker = WorkerInfo {
            id: Uuid::new_v4(),
            name: registration.name,
            labels: registration.labels,
            capacity: normalize_capacity(registration.capacity),
            state: WorkerState::Online,
            registered_at: now,
            last_seen_at: now,
        };
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        state.workers.insert(worker.id, worker.clone());
        push_log(
            &mut state,
            NewLog {
                level: LogLevel::Info,
                source: LogSource::Controller,
                message: format!("worker registered: {}", worker.name),
                job_id: None,
                task_id: None,
                worker_id: Some(worker.id),
                stream: None,
            },
        );
        Ok(worker)
    }

    pub fn record_worker_logs(
        &self,
        worker_id: WorkerId,
        batch: WorkerLogBatch,
    ) -> Result<Vec<FarmLogEntry>, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        if !state.workers.contains_key(&worker_id) {
            return Err(FarmError::WorkerNotFound(worker_id));
        }
        let mut recorded = Vec::new();
        for input in batch.entries {
            let entry = push_log(
                &mut state,
                NewLog {
                    level: input.level,
                    source: LogSource::Worker,
                    message: input.message,
                    job_id: input.job_id,
                    task_id: input.task_id,
                    worker_id: Some(worker_id),
                    stream: input.stream,
                },
            );
            recorded.push(entry);
        }
        Ok(recorded)
    }

    pub fn heartbeat_worker(&self, worker_id: WorkerId) -> Result<WorkerInfo, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let worker = state
            .workers
            .get_mut(&worker_id)
            .ok_or(FarmError::WorkerNotFound(worker_id))?;
        worker.state = WorkerState::Online;
        worker.last_seen_at = Utc::now();
        Ok(worker.clone())
    }

    pub fn pause_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        match job.state {
            JobState::Queued | JobState::Running => {
                job.state = JobState::Paused;
                job.updated_at = Utc::now();
            }
            JobState::Paused => {}
            JobState::Succeeded | JobState::Failed | JobState::Cancelled => {
                return Err(FarmError::InvalidState(format!(
                    "job '{}' cannot be paused from state {:?}",
                    job.id, job.state
                )));
            }
        }
        Ok(job.clone())
    }

    pub fn resume_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        match job.state {
            JobState::Paused => update_job_state(job),
            JobState::Queued | JobState::Running => {}
            JobState::Succeeded | JobState::Failed | JobState::Cancelled => {
                return Err(FarmError::InvalidState(format!(
                    "job '{}' cannot be resumed from state {:?}",
                    job.id, job.state
                )));
            }
        }
        Ok(job.clone())
    }

    pub fn cancel_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        if job.state == JobState::Cancelled {
            return Ok(job.clone());
        }
        if job.state == JobState::Succeeded {
            return Err(FarmError::InvalidState(format!(
                "job '{}' cannot be cancelled after success",
                job.id
            )));
        }
        let now = Utc::now();
        job.state = JobState::Cancelled;
        job.updated_at = now;
        for task in &mut job.tasks {
            if !matches!(
                task.state,
                TaskState::Succeeded | TaskState::Failed | TaskState::Cancelled
            ) {
                task.state = TaskState::Cancelled;
                task.updated_at = now;
                task.completed_at = Some(now);
                if task.lease.is_none() {
                    task.started_at = None;
                }
            }
        }
        Ok(job.clone())
    }

    pub fn update_job_priority(&self, job_id: JobId, priority: i32) -> Result<Job, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        job.priority = priority;
        job.updated_at = Utc::now();
        Ok(job.clone())
    }

    pub fn lease_task(&self, worker_id: WorkerId) -> Result<Option<TaskLease>, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let worker = state
            .workers
            .get(&worker_id)
            .cloned()
            .ok_or(FarmError::WorkerNotFound(worker_id))?;

        expire_old_leases(&mut state);

        if active_worker_slots(&state, worker_id) >= worker.capacity.slots {
            return Ok(None);
        }

        let now = Utc::now();
        let selected = state
            .jobs
            .values()
            .filter(|job| matches!(job.state, JobState::Queued | JobState::Running))
            .flat_map(|job| {
                job.tasks
                    .iter()
                    .filter(|task| task.state == TaskState::Pending)
                    .map(move |task| {
                        (
                            job.id,
                            task.id,
                            job.priority,
                            task.created_at.timestamp_millis(),
                        )
                    })
            })
            .filter(|(job_id, task_id, _, _)| dependencies_are_satisfied(&state, *job_id, *task_id))
            .filter(|(job_id, task_id, _, _)| {
                task_matches_worker(&state, *job_id, *task_id, &worker)
            })
            .max_by_key(|(_, _, priority, created_at)| (*priority, std::cmp::Reverse(*created_at)));

        let Some((job_id, task_id, _, _)) = selected else {
            return Ok(None);
        };

        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        let lease_token = Uuid::new_v4().to_string();
        let leased_task = {
            let task = job
                .tasks
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or(FarmError::TaskNotFound(task_id))?;

            task.state = TaskState::Leased;
            task.attempts += 1;
            task.updated_at = now;
            task.lease = Some(TaskLeaseInfo {
                token: lease_token.clone(),
                worker_id,
                leased_at: now,
                expires_at: now + self.lease_ttl(),
            });
            task.clone()
        };
        let task_name = leased_task.name.clone();
        update_job_state(job);
        push_log(
            &mut state,
            NewLog {
                level: LogLevel::Info,
                source: LogSource::Controller,
                message: format!("leased task: {task_name}"),
                job_id: Some(job_id),
                task_id: Some(task_id),
                worker_id: Some(worker_id),
                stream: None,
            },
        );

        Ok(Some(TaskLease {
            task: leased_task,
            lease_token,
        }))
    }

    pub fn cancel_task(&self, task_id: TaskId) -> Result<Task, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let (job_id, task_index) = find_task_location(&state, task_id)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        let task = job
            .tasks
            .get_mut(task_index)
            .ok_or(FarmError::TaskNotFound(task_id))?;
        match task.state {
            TaskState::Pending | TaskState::Leased | TaskState::Running => {
                task.state = TaskState::Cancelled;
                task.updated_at = Utc::now();
                task.completed_at = Some(Utc::now());
                if task.lease.is_none() {
                    task.started_at = None;
                }
            }
            TaskState::Cancelled => {}
            TaskState::Succeeded | TaskState::Failed => {
                return Err(FarmError::InvalidState(format!(
                    "task '{}' cannot be cancelled from state {:?}",
                    task.id, task.state
                )));
            }
        }
        let task = task.clone();
        update_job_state(job);
        Ok(task)
    }

    pub fn requeue_task(&self, task_id: TaskId) -> Result<Task, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let (job_id, task_index) = find_task_location(&state, task_id)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        let task = job
            .tasks
            .get_mut(task_index)
            .ok_or(FarmError::TaskNotFound(task_id))?;
        match task.state {
            TaskState::Pending | TaskState::Failed | TaskState::Cancelled => {
                task.state = TaskState::Pending;
                task.attempts = 0;
                task.lease = None;
                task.started_at = None;
                task.completed_at = None;
                task.last_exit_code = None;
                task.updated_at = Utc::now();
            }
            TaskState::Leased | TaskState::Running | TaskState::Succeeded => {
                return Err(FarmError::InvalidState(format!(
                    "task '{}' cannot be requeued from state {:?}",
                    task.id, task.state
                )));
            }
        }
        let task = task.clone();
        update_job_state(job);
        Ok(task)
    }

    pub fn mark_task_started(
        &self,
        task_id: TaskId,
        started: TaskStarted,
    ) -> Result<Task, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let (job_id, task_index) = find_task_location(&state, task_id)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        let task = job
            .tasks
            .get_mut(task_index)
            .ok_or(FarmError::TaskNotFound(task_id))?;
        validate_lease(task, started.worker_id, &started.lease_token)?;
        task.state = TaskState::Running;
        task.started_at = Some(Utc::now());
        task.updated_at = Utc::now();
        let task = task.clone();
        update_job_state(job);
        push_log(
            &mut state,
            NewLog {
                level: LogLevel::Info,
                source: LogSource::Controller,
                message: format!("task started: {}", task.name),
                job_id: Some(job_id),
                task_id: Some(task.id),
                worker_id: Some(started.worker_id),
                stream: None,
            },
        );
        Ok(task)
    }

    pub fn renew_task_lease(
        &self,
        task_id: TaskId,
        renewal: TaskLeaseRenewal,
    ) -> Result<Task, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let (job_id, task_index) = find_task_location(&state, task_id)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        let task = job
            .tasks
            .get_mut(task_index)
            .ok_or(FarmError::TaskNotFound(task_id))?;
        validate_lease(task, renewal.worker_id, &renewal.lease_token)?;

        let now = Utc::now();
        if let Some(lease) = &mut task.lease {
            lease.expires_at = now + self.lease_ttl();
        }
        task.updated_at = now;
        Ok(task.clone())
    }

    pub fn complete_task(
        &self,
        task_id: TaskId,
        completion: TaskComplete,
    ) -> Result<Task, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let (job_id, task_index) = find_task_location(&state, task_id)?;
        let job = state
            .jobs
            .get_mut(&job_id)
            .ok_or(FarmError::JobNotFound(job_id))?;
        let task = job
            .tasks
            .get_mut(task_index)
            .ok_or(FarmError::TaskNotFound(task_id))?;
        validate_lease(task, completion.worker_id, &completion.lease_token)?;
        let job_was_paused = job.state == JobState::Paused;

        if task.state == TaskState::Cancelled {
            task.completed_at = Some(Utc::now());
            task.updated_at = Utc::now();
            task.lease = None;
            return Ok(task.clone());
        }

        task.completed_at = Some(Utc::now());
        task.updated_at = Utc::now();
        task.last_exit_code = Some(completion.exit_code);
        task.stdout_tail = completion.stdout_tail;
        task.stderr_tail = completion.stderr_tail;
        task.artifacts = completion.artifacts;
        task.lease = None;
        task.state = if completion.exit_code == 0 {
            TaskState::Succeeded
        } else if task.attempts <= task.max_retries {
            task.started_at = None;
            task.completed_at = None;
            TaskState::Pending
        } else {
            TaskState::Failed
        };

        let task = task.clone();
        update_job_state(job);
        if job_was_paused && matches!(job.state, JobState::Queued | JobState::Running) {
            job.state = JobState::Paused;
        }
        let level = if completion.exit_code == 0 {
            LogLevel::Info
        } else {
            LogLevel::Error
        };
        push_log(
            &mut state,
            NewLog {
                level,
                source: LogSource::Controller,
                message: format!(
                    "task completed: {} (exit {})",
                    task.name, completion.exit_code
                ),
                job_id: Some(job_id),
                task_id: Some(task.id),
                worker_id: Some(completion.worker_id),
                stream: None,
            },
        );
        Ok(task)
    }

    fn lease_ttl(&self) -> Duration {
        Duration::seconds(self.config.lease_ttl_seconds.max(1))
    }

    pub fn get_task_artifact(
        &self,
        task_id: TaskId,
        artifact_index: usize,
    ) -> Result<TaskArtifact, FarmError> {
        let state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        let (job_id, task_index) = find_task_location(&state, task_id)?;
        let task = state
            .jobs
            .get(&job_id)
            .and_then(|job| job.tasks.get(task_index))
            .ok_or(FarmError::TaskNotFound(task_id))?;
        task.artifacts
            .get(artifact_index)
            .cloned()
            .ok_or(FarmError::ArtifactNotFound {
                task_id,
                artifact_index,
            })
    }
}

#[derive(Debug, Clone)]
pub struct SqliteScheduler {
    inner: InMemoryScheduler,
    database_path: Arc<PathBuf>,
    persist_lock: Arc<Mutex<()>>,
}

impl SqliteScheduler {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, FarmError> {
        Self::open_with_config(path, SchedulerConfig::default())
    }

    pub fn open_with_config(
        path: impl AsRef<Path>,
        config: SchedulerConfig,
    ) -> Result<Self, FarmError> {
        let database_path = Arc::new(path.as_ref().to_path_buf());
        let connection = Connection::open(database_path.as_ref()).map_err(storage_error)?;
        initialize_sqlite(&connection)?;
        let state = load_sqlite_state(&connection)?;
        Ok(Self {
            inner: InMemoryScheduler::from_state(state, config),
            database_path,
            persist_lock: Arc::default(),
        })
    }

    pub fn submit_job(&self, submission: JobSubmit) -> Result<Job, FarmError> {
        self.write_durably(|inner| inner.submit_job(submission))
    }

    pub fn get_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        self.inner.get_job(job_id)
    }

    pub fn list_jobs(&self) -> Result<Vec<Job>, FarmError> {
        self.inner.list_jobs()
    }

    pub fn list_workers(&self) -> Result<Vec<WorkerInfo>, FarmError> {
        self.inner.list_workers()
    }

    pub fn dashboard_snapshot(&self) -> Result<DashboardSnapshot, FarmError> {
        self.inner.dashboard_snapshot()
    }

    pub fn list_logs(&self) -> Result<Vec<FarmLogEntry>, FarmError> {
        self.inner.list_logs()
    }

    pub fn list_worker_logs(&self, worker_id: WorkerId) -> Result<Vec<FarmLogEntry>, FarmError> {
        self.inner.list_worker_logs(worker_id)
    }

    pub fn record_worker_logs(
        &self,
        worker_id: WorkerId,
        batch: WorkerLogBatch,
    ) -> Result<Vec<FarmLogEntry>, FarmError> {
        self.write_durably(|inner| inner.record_worker_logs(worker_id, batch))
    }

    pub fn pause_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        self.write_durably(|inner| inner.pause_job(job_id))
    }

    pub fn resume_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        self.write_durably(|inner| inner.resume_job(job_id))
    }

    pub fn cancel_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        self.write_durably(|inner| inner.cancel_job(job_id))
    }

    pub fn update_job_priority(&self, job_id: JobId, priority: i32) -> Result<Job, FarmError> {
        self.write_durably(|inner| inner.update_job_priority(job_id, priority))
    }

    pub fn cancel_task(&self, task_id: TaskId) -> Result<Task, FarmError> {
        self.write_durably(|inner| inner.cancel_task(task_id))
    }

    pub fn requeue_task(&self, task_id: TaskId) -> Result<Task, FarmError> {
        self.write_durably(|inner| inner.requeue_task(task_id))
    }

    pub fn register_worker(&self, registration: WorkerRegister) -> Result<WorkerInfo, FarmError> {
        self.write_durably(|inner| inner.register_worker(registration))
    }

    pub fn heartbeat_worker(&self, worker_id: WorkerId) -> Result<WorkerInfo, FarmError> {
        self.write_durably(|inner| inner.heartbeat_worker(worker_id))
    }

    pub fn lease_task(&self, worker_id: WorkerId) -> Result<Option<TaskLease>, FarmError> {
        self.write_durably(|inner| inner.lease_task(worker_id))
    }

    pub fn mark_task_started(
        &self,
        task_id: TaskId,
        started: TaskStarted,
    ) -> Result<Task, FarmError> {
        self.write_durably(|inner| inner.mark_task_started(task_id, started))
    }

    pub fn renew_task_lease(
        &self,
        task_id: TaskId,
        renewal: TaskLeaseRenewal,
    ) -> Result<Task, FarmError> {
        self.write_durably(|inner| inner.renew_task_lease(task_id, renewal))
    }

    pub fn complete_task(
        &self,
        task_id: TaskId,
        completion: TaskComplete,
    ) -> Result<Task, FarmError> {
        self.write_durably(|inner| inner.complete_task(task_id, completion))
    }

    pub fn get_task_artifact(
        &self,
        task_id: TaskId,
        artifact_index: usize,
    ) -> Result<TaskArtifact, FarmError> {
        self.inner.get_task_artifact(task_id, artifact_index)
    }

    fn write_durably<T>(
        &self,
        operation: impl FnOnce(&InMemoryScheduler) -> Result<T, FarmError>,
    ) -> Result<T, FarmError> {
        let previous = self.inner.state_snapshot()?;
        let value = operation(&self.inner)?;
        if let Err(error) = self.persist() {
            self.inner.replace_state(previous)?;
            return Err(error);
        }
        Ok(value)
    }

    fn persist(&self) -> Result<(), FarmError> {
        let _guard = self
            .persist_lock
            .lock()
            .map_err(|_| FarmError::LockPoisoned)?;
        let state = self.inner.state_snapshot()?;
        let connection = Connection::open(self.database_path.as_ref()).map_err(storage_error)?;
        save_sqlite_state(&connection, &state)
    }
}

fn initialize_sqlite(connection: &Connection) -> Result<(), FarmError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS scheduler_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                state_json TEXT NOT NULL
            );",
        )
        .map_err(storage_error)?;
    let state_exists = connection
        .query_row("SELECT 1 FROM scheduler_state WHERE id = 1", [], |_| Ok(()))
        .optional()
        .map_err(storage_error)?
        .is_some();
    if !state_exists {
        save_sqlite_state(connection, &SchedulerState::default())?;
    }
    Ok(())
}

fn load_sqlite_state(connection: &Connection) -> Result<SchedulerState, FarmError> {
    let state_json: String = connection
        .query_row(
            "SELECT state_json FROM scheduler_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .map_err(storage_error)?;
    serde_json::from_str(&state_json).map_err(storage_error)
}

fn save_sqlite_state(connection: &Connection, state: &SchedulerState) -> Result<(), FarmError> {
    let state_json = serde_json::to_string(state).map_err(storage_error)?;
    connection
        .execute(
            "INSERT INTO scheduler_state (id, state_json)
             VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET state_json = excluded.state_json",
            params![state_json],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn storage_error(error: impl std::fmt::Display) -> FarmError {
    FarmError::Storage(error.to_string())
}

fn normalize_capacity(capacity: WorkerCapacity) -> WorkerCapacity {
    WorkerCapacity {
        slots: capacity.slots.max(1),
    }
}

fn task_from_submit(
    job_id: JobId,
    task_ids_by_name: &HashMap<String, TaskId>,
    submission: TaskSubmit,
    job_max_retries: u32,
    now: chrono::DateTime<Utc>,
) -> Result<Task, FarmError> {
    let id = *task_ids_by_name
        .get(&submission.name)
        .expect("task id should exist for every task");
    let dependencies = submission
        .dependencies
        .iter()
        .map(|dependency| {
            task_ids_by_name.get(dependency).copied().ok_or_else(|| {
                FarmError::InvalidSubmission(format!(
                    "task '{}' depends on unknown task '{}'",
                    submission.name, dependency
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Task {
        id,
        job_id,
        name: submission.name,
        state: TaskState::Pending,
        command: submission.command,
        openjd: submission.openjd,
        dependencies,
        requirements: submission.requirements,
        attempts: 0,
        max_retries: submission.max_retries.unwrap_or(job_max_retries),
        lease: None,
        artifact_paths: submission.artifact_paths,
        artifacts: Vec::new(),
        created_at: now,
        updated_at: now,
        started_at: None,
        completed_at: None,
        last_exit_code: None,
        stdout_tail: None,
        stderr_tail: None,
    })
}

fn active_worker_slots(state: &SchedulerState, worker_id: WorkerId) -> u32 {
    state
        .jobs
        .values()
        .flat_map(|job| &job.tasks)
        .filter(|task| matches!(task.state, TaskState::Leased | TaskState::Running))
        .filter(|task| {
            task.lease
                .as_ref()
                .is_some_and(|lease| lease.worker_id == worker_id)
        })
        .count() as u32
}

fn task_matches_worker(
    state: &SchedulerState,
    job_id: JobId,
    task_id: TaskId,
    worker: &WorkerInfo,
) -> bool {
    let Some(job) = state.jobs.get(&job_id) else {
        return false;
    };
    let Some(task) = job.tasks.iter().find(|task| task.id == task_id) else {
        return false;
    };

    let pool_matches = task.requirements.pools.is_empty()
        || worker.labels.get("pool").is_some_and(|pool| {
            task.requirements
                .pools
                .iter()
                .any(|required| required == pool)
        });
    let labels_match = task.requirements.labels.iter().all(|(key, required)| {
        worker
            .labels
            .get(key)
            .is_some_and(|actual| label_value_matches(actual, required))
    });

    pool_matches && labels_match
}

fn label_value_matches(actual: &str, required: &str) -> bool {
    actual == required
        || actual
            .split(',')
            .map(str::trim)
            .any(|candidate| candidate == required)
}

fn dependencies_are_satisfied(state: &SchedulerState, job_id: JobId, task_id: TaskId) -> bool {
    let Some(job) = state.jobs.get(&job_id) else {
        return false;
    };
    let Some(task) = job.tasks.iter().find(|task| task.id == task_id) else {
        return false;
    };
    task.dependencies.iter().all(|dependency_id| {
        job.tasks.iter().any(|candidate| {
            candidate.id == *dependency_id && candidate.state == TaskState::Succeeded
        })
    })
}

fn expire_old_leases(state: &mut SchedulerState) {
    let now = Utc::now();
    for job in state.jobs.values_mut() {
        let mut changed = false;
        for task in &mut job.tasks {
            if matches!(task.state, TaskState::Leased | TaskState::Running)
                && task
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.expires_at < now)
            {
                task.state = TaskState::Pending;
                task.lease = None;
                task.started_at = None;
                task.updated_at = now;
                changed = true;
            }
        }
        if changed {
            update_job_state(job);
        }
    }
}

struct NewLog {
    level: LogLevel,
    source: LogSource,
    message: String,
    job_id: Option<JobId>,
    task_id: Option<TaskId>,
    worker_id: Option<WorkerId>,
    stream: Option<String>,
}

fn push_log(state: &mut SchedulerState, log: NewLog) -> FarmLogEntry {
    let entry = FarmLogEntry {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        level: log.level,
        source: log.source,
        stream: log.stream,
        message: log.message,
        job_id: log.job_id,
        task_id: log.task_id,
        worker_id: log.worker_id,
    };
    state.logs.push(entry.clone());
    if state.logs.len() > MAX_LOG_ENTRIES {
        let overflow = state.logs.len() - MAX_LOG_ENTRIES;
        state.logs.drain(0..overflow);
    }
    entry
}

fn find_task_location(
    state: &SchedulerState,
    task_id: TaskId,
) -> Result<(JobId, usize), FarmError> {
    for job in state.jobs.values() {
        if let Some(index) = job.tasks.iter().position(|task| task.id == task_id) {
            return Ok((job.id, index));
        }
    }
    Err(FarmError::TaskNotFound(task_id))
}

fn validate_lease(task: &Task, worker_id: WorkerId, lease_token: &str) -> Result<(), FarmError> {
    let Some(lease) = &task.lease else {
        return Err(FarmError::InvalidLease);
    };
    if lease.worker_id != worker_id || lease.token != lease_token || lease.expires_at < Utc::now() {
        return Err(FarmError::InvalidLease);
    }
    Ok(())
}

fn update_job_state(job: &mut Job) {
    job.updated_at = Utc::now();
    job.state = if job.tasks.iter().any(|task| task.state == TaskState::Failed) {
        JobState::Failed
    } else if job
        .tasks
        .iter()
        .all(|task| task.state == TaskState::Cancelled)
    {
        JobState::Cancelled
    } else if job
        .tasks
        .iter()
        .all(|task| task.state == TaskState::Succeeded)
    {
        JobState::Succeeded
    } else if job.tasks.iter().any(|task| {
        matches!(
            task.state,
            TaskState::Leased | TaskState::Running | TaskState::Succeeded
        )
    }) {
        JobState::Running
    } else {
        JobState::Queued
    };
}

fn compute_stats(state: &SchedulerState) -> FarmStats {
    let mut stats = FarmStats {
        jobs_total: state.jobs.len(),
        workers_total: state.workers.len(),
        ..FarmStats::default()
    };

    for job in state.jobs.values() {
        match job.state {
            JobState::Queued => stats.jobs_queued += 1,
            JobState::Running => stats.jobs_running += 1,
            JobState::Paused => stats.jobs_paused += 1,
            JobState::Succeeded => stats.jobs_succeeded += 1,
            JobState::Failed => stats.jobs_failed += 1,
            JobState::Cancelled => {}
        }
        for task in &job.tasks {
            stats.tasks_total += 1;
            match task.state {
                TaskState::Pending => stats.tasks_pending += 1,
                TaskState::Leased => stats.tasks_leased += 1,
                TaskState::Running => stats.tasks_running += 1,
                TaskState::Succeeded => stats.tasks_succeeded += 1,
                TaskState::Failed => stats.tasks_failed += 1,
                TaskState::Cancelled => {}
            }
        }
    }

    for worker in state.workers.values() {
        stats.worker_slots += worker.capacity.slots;
        let used_slots = active_worker_slots(state, worker.id);
        stats.worker_slots_used += used_slots;
        stats.worker_slots_available += worker.capacity.slots.saturating_sub(used_slots);
        match worker.state {
            WorkerState::Online => stats.workers_online += 1,
            WorkerState::Offline => stats.workers_offline += 1,
        }
    }

    stats
}

#[cfg(test)]
mod tests {
    use crate::models::{
        CommandSpec, TaskLease, TaskLeaseRenewal, TaskRequirements, WorkerCapacity, WorkerId,
        WorkerInfo,
    };

    use super::*;

    #[test]
    fn leases_dependency_tasks_in_order() {
        let scheduler = InMemoryScheduler::default();
        let job = scheduler
            .submit_job(JobSubmit {
                name: "dependency-demo".to_string(),
                priority: 0,
                max_retries: 0,
                openjd: None,
                tasks: vec![
                    TaskSubmit {
                        name: "prepare".to_string(),
                        command: command("echo"),
                        dependencies: vec![],
                        requirements: Default::default(),
                        max_retries: None,
                        artifact_paths: Vec::new(),
                        openjd: None,
                    },
                    TaskSubmit {
                        name: "render".to_string(),
                        command: command("echo"),
                        dependencies: vec!["prepare".to_string()],
                        requirements: Default::default(),
                        max_retries: None,
                        artifact_paths: Vec::new(),
                        openjd: None,
                    },
                ],
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "local".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity::default(),
            })
            .expect("worker should register");

        let first = scheduler
            .lease_task(worker.id)
            .expect("lease should work")
            .expect("a task should be leased");
        assert_eq!(first.task.name, "prepare");
        scheduler
            .complete_task(
                first.task.id,
                TaskComplete {
                    worker_id: worker.id,
                    lease_token: first.lease_token,
                    exit_code: 0,
                    stdout_tail: None,
                    stderr_tail: None,
                    artifacts: Vec::new(),
                },
            )
            .expect("completion should work");
        let second = scheduler
            .lease_task(worker.id)
            .expect("lease should work")
            .expect("next task should be leased");
        assert_eq!(second.task.name, "render");
        assert_eq!(scheduler.get_job(job.id).unwrap().state, JobState::Running);
    }

    #[test]
    fn dashboard_snapshot_reports_queue_and_worker_stats() {
        let scheduler = InMemoryScheduler::default();
        scheduler
            .submit_job(JobSubmit {
                name: "stats-demo".to_string(),
                priority: 3,
                max_retries: 0,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        scheduler
            .register_worker(WorkerRegister {
                name: "render-node-01".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity { slots: 4 },
            })
            .expect("worker should register");

        let snapshot = scheduler
            .dashboard_snapshot()
            .expect("snapshot should be available");
        assert_eq!(snapshot.stats.jobs_total, 1);
        assert_eq!(snapshot.stats.jobs_queued, 1);
        assert_eq!(snapshot.stats.tasks_pending, 1);
        assert_eq!(snapshot.stats.workers_online, 1);
        assert_eq!(snapshot.stats.worker_slots, 4);
        assert_eq!(snapshot.stats.worker_slots_available, 4);
        assert_eq!(snapshot.jobs[0].name, "stats-demo");
        assert_eq!(snapshot.workers[0].name, "render-node-01");
    }

    #[test]
    fn renews_task_lease_before_it_expires() {
        let scheduler = InMemoryScheduler::with_config(SchedulerConfig {
            lease_ttl_seconds: 300,
        });
        let worker = register_worker(&scheduler);
        let lease = lease_single_task(&scheduler, worker.id);
        let original_expires_at = lease.task.lease.as_ref().unwrap().expires_at;

        let renewed = scheduler
            .renew_task_lease(
                lease.task.id,
                TaskLeaseRenewal {
                    worker_id: worker.id,
                    lease_token: lease.lease_token,
                },
            )
            .expect("lease should renew");

        assert_eq!(renewed.state, TaskState::Leased);
        assert!(renewed.lease.unwrap().expires_at >= original_expires_at);
    }

    #[test]
    fn rejects_stale_task_lease_renewal() {
        let scheduler = InMemoryScheduler::with_config(SchedulerConfig {
            lease_ttl_seconds: 300,
        });
        let worker = register_worker(&scheduler);
        let lease = lease_single_task(&scheduler, worker.id);

        {
            let mut state = scheduler.inner.lock().unwrap();
            let job = state.jobs.values_mut().next().unwrap();
            let task = job
                .tasks
                .iter_mut()
                .find(|task| task.id == lease.task.id)
                .unwrap();
            task.lease.as_mut().unwrap().expires_at = Utc::now() - Duration::seconds(1);
        }

        let result = scheduler.renew_task_lease(
            lease.task.id,
            TaskLeaseRenewal {
                worker_id: worker.id,
                lease_token: lease.lease_token,
            },
        );

        assert!(matches!(result, Err(FarmError::InvalidLease)));
    }

    #[test]
    fn sqlite_scheduler_restores_jobs_workers_and_retry_state() {
        let database_path = temp_database_path();
        let scheduler = SqliteScheduler::open(&database_path).expect("sqlite should open");
        let job = scheduler
            .submit_job(JobSubmit {
                name: "durable-demo".to_string(),
                priority: 5,
                max_retries: 2,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "render-node-01".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity { slots: 2 },
            })
            .expect("worker should register");
        let lease = scheduler
            .lease_task(worker.id)
            .expect("lease should work")
            .expect("a task should be leased");
        scheduler
            .complete_task(
                lease.task.id,
                TaskComplete {
                    worker_id: worker.id,
                    lease_token: lease.lease_token,
                    exit_code: 1,
                    stdout_tail: Some("stdout".to_string()),
                    stderr_tail: Some("stderr".to_string()),
                    artifacts: Vec::new(),
                },
            )
            .expect("failed task should requeue");

        let reopened = SqliteScheduler::open(&database_path).expect("sqlite should reopen");
        let restored = reopened.get_job(job.id).expect("job should be durable");
        assert_eq!(restored.priority, 5);
        assert_eq!(restored.tasks[0].attempts, 1);
        assert_eq!(restored.tasks[0].state, TaskState::Pending);
        assert_eq!(restored.tasks[0].stdout_tail.as_deref(), Some("stdout"));
        assert_eq!(reopened.list_workers().unwrap()[0].name, "render-node-01");
        let _ = std::fs::remove_file(database_path);
    }

    #[test]
    fn sqlite_scheduler_recovers_expired_leases_after_restart() {
        let database_path = temp_database_path();
        let scheduler = SqliteScheduler::open(&database_path).expect("sqlite should open");
        scheduler
            .submit_job(JobSubmit {
                name: "lease-recovery".to_string(),
                priority: 0,
                max_retries: 0,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "worker".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity::default(),
            })
            .expect("worker should register");
        let lease = scheduler
            .lease_task(worker.id)
            .expect("lease should work")
            .expect("a task should be leased");

        {
            let mut state = scheduler.inner.inner.lock().unwrap();
            let job = state.jobs.values_mut().next().unwrap();
            let task = job
                .tasks
                .iter_mut()
                .find(|task| task.id == lease.task.id)
                .unwrap();
            task.state = TaskState::Running;
            task.lease.as_mut().unwrap().expires_at = Utc::now() - Duration::seconds(1);
        }
        scheduler.persist().expect("expired state should persist");

        let reopened = SqliteScheduler::open(&database_path).expect("sqlite should reopen");
        let recovered = reopened
            .lease_task(worker.id)
            .expect("lease should recover expired work")
            .expect("expired task should be available again");
        assert_eq!(recovered.task.id, lease.task.id);
        assert_eq!(recovered.task.attempts, 2);
        let _ = std::fs::remove_file(database_path);
    }

    #[test]
    fn worker_slots_limit_concurrent_leases() {
        let scheduler = InMemoryScheduler::default();
        scheduler
            .submit_job(JobSubmit {
                name: "slots-demo".to_string(),
                priority: 0,
                max_retries: 0,
                tasks: (1..=3)
                    .map(|index| TaskSubmit {
                        name: format!("task-{index}"),
                        command: command("echo"),
                        dependencies: vec![],
                        requirements: Default::default(),
                        max_retries: None,
                        artifact_paths: Vec::new(),
                        openjd: None,
                    })
                    .collect(),
                openjd: None,
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "multi-slot".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity { slots: 2 },
            })
            .expect("worker should register");

        let first = scheduler.lease_task(worker.id).unwrap().unwrap();
        let second = scheduler.lease_task(worker.id).unwrap().unwrap();
        assert_ne!(first.task.id, second.task.id);
        assert!(scheduler.lease_task(worker.id).unwrap().is_none());

        let snapshot = scheduler.dashboard_snapshot().unwrap();
        assert_eq!(snapshot.stats.worker_slots, 2);
        assert_eq!(snapshot.stats.worker_slots_used, 2);
        assert_eq!(snapshot.stats.worker_slots_available, 0);
    }

    #[test]
    fn worker_slot_is_released_after_task_completion() {
        let scheduler = InMemoryScheduler::default();
        scheduler
            .submit_job(JobSubmit {
                name: "slot-release".to_string(),
                priority: 0,
                max_retries: 0,
                tasks: vec![
                    TaskSubmit {
                        name: "first".to_string(),
                        command: command("echo"),
                        dependencies: vec![],
                        requirements: Default::default(),
                        max_retries: None,
                        artifact_paths: Vec::new(),
                        openjd: None,
                    },
                    TaskSubmit {
                        name: "second".to_string(),
                        command: command("echo"),
                        dependencies: vec![],
                        requirements: Default::default(),
                        max_retries: None,
                        artifact_paths: Vec::new(),
                        openjd: None,
                    },
                ],
                openjd: None,
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "single-slot".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity { slots: 1 },
            })
            .expect("worker should register");

        let first = scheduler.lease_task(worker.id).unwrap().unwrap();
        assert!(scheduler.lease_task(worker.id).unwrap().is_none());
        scheduler
            .complete_task(
                first.task.id,
                TaskComplete {
                    worker_id: worker.id,
                    lease_token: first.lease_token,
                    exit_code: 0,
                    stdout_tail: None,
                    stderr_tail: None,
                    artifacts: Vec::new(),
                },
            )
            .expect("completion should release the slot");

        let second = scheduler.lease_task(worker.id).unwrap().unwrap();
        assert_ne!(first.task.id, second.task.id);
        assert_eq!(
            scheduler
                .dashboard_snapshot()
                .unwrap()
                .stats
                .worker_slots_used,
            1
        );
    }

    #[test]
    fn leases_only_tasks_matching_worker_labels_and_pool() {
        let scheduler = InMemoryScheduler::default();
        scheduler
            .submit_job(JobSubmit {
                name: "routing-demo".to_string(),
                priority: 0,
                max_retries: 0,
                openjd: None,
                tasks: vec![
                    TaskSubmit {
                        name: "windows-render".to_string(),
                        command: command("echo"),
                        dependencies: vec![],
                        requirements: TaskRequirements {
                            labels: HashMap::from([
                                ("os".to_string(), "windows".to_string()),
                                ("app".to_string(), "maya".to_string()),
                            ]),
                            pools: vec!["lighting".to_string()],
                        },
                        max_retries: None,
                        artifact_paths: Vec::new(),
                        openjd: None,
                    },
                    TaskSubmit {
                        name: "linux-sim".to_string(),
                        command: command("echo"),
                        dependencies: vec![],
                        requirements: TaskRequirements {
                            labels: HashMap::from([("os".to_string(), "linux".to_string())]),
                            pools: vec!["sim".to_string()],
                        },
                        max_retries: None,
                        artifact_paths: Vec::new(),
                        openjd: None,
                    },
                ],
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "maya-node".to_string(),
                labels: HashMap::from([
                    ("os".to_string(), "windows".to_string()),
                    ("app".to_string(), "maya,blender".to_string()),
                    ("pool".to_string(), "lighting".to_string()),
                ]),
                capacity: WorkerCapacity::default(),
            })
            .expect("worker should register");

        let lease = scheduler
            .lease_task(worker.id)
            .expect("lease should work")
            .expect("matching task should be leased");
        assert_eq!(lease.task.name, "windows-render");
        assert!(scheduler.lease_task(worker.id).unwrap().is_none());
    }

    #[test]
    fn leases_unconstrained_tasks_to_unlabeled_workers() {
        let scheduler = InMemoryScheduler::default();
        scheduler
            .submit_job(JobSubmit {
                name: "default-routing".to_string(),
                priority: 0,
                max_retries: 0,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "plain-worker".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity::default(),
            })
            .expect("worker should register");

        assert!(scheduler.lease_task(worker.id).unwrap().is_some());
    }

    #[test]
    fn paused_jobs_do_not_lease_until_resumed() {
        let scheduler = InMemoryScheduler::default();
        let job = scheduler
            .submit_job(JobSubmit {
                name: "pause-demo".to_string(),
                priority: 0,
                max_retries: 0,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "local".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity::default(),
            })
            .expect("worker should register");

        assert_eq!(scheduler.pause_job(job.id).unwrap().state, JobState::Paused);
        assert!(scheduler.lease_task(worker.id).unwrap().is_none());
        assert_eq!(
            scheduler.resume_job(job.id).unwrap().state,
            JobState::Queued
        );
        assert!(scheduler.lease_task(worker.id).unwrap().is_some());
    }

    #[test]
    fn cancel_and_requeue_task_are_idempotent_state_transitions() {
        let scheduler = InMemoryScheduler::default();
        scheduler
            .submit_job(JobSubmit {
                name: "task-actions".to_string(),
                priority: 0,
                max_retries: 0,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        let task_id = scheduler.list_jobs().unwrap()[0].tasks[0].id;

        assert_eq!(
            scheduler.cancel_task(task_id).unwrap().state,
            TaskState::Cancelled
        );
        assert_eq!(
            scheduler.cancel_task(task_id).unwrap().state,
            TaskState::Cancelled
        );
        let requeued = scheduler.requeue_task(task_id).unwrap();
        assert_eq!(requeued.state, TaskState::Pending);
        assert_eq!(requeued.attempts, 0);
    }

    #[test]
    fn failed_tasks_can_be_requeued_and_priority_can_change() {
        let scheduler = InMemoryScheduler::default();
        let job = scheduler
            .submit_job(JobSubmit {
                name: "retry-demo".to_string(),
                priority: 1,
                max_retries: 0,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        let worker = scheduler
            .register_worker(WorkerRegister {
                name: "local".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity::default(),
            })
            .expect("worker should register");
        let lease = scheduler.lease_task(worker.id).unwrap().unwrap();
        scheduler
            .complete_task(
                lease.task.id,
                TaskComplete {
                    worker_id: worker.id,
                    lease_token: lease.lease_token,
                    exit_code: 1,
                    stdout_tail: None,
                    stderr_tail: Some("failed".to_string()),
                    artifacts: Vec::new(),
                },
            )
            .expect("task should fail");

        assert_eq!(scheduler.get_job(job.id).unwrap().state, JobState::Failed);
        assert_eq!(
            scheduler.requeue_task(lease.task.id).unwrap().state,
            TaskState::Pending
        );
        assert_eq!(scheduler.get_job(job.id).unwrap().state, JobState::Queued);
        assert_eq!(
            scheduler.update_job_priority(job.id, 10).unwrap().priority,
            10
        );
    }
    fn command(executable: &str) -> CommandSpec {
        CommandSpec {
            executable: executable.to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
            timeout_seconds: None,
        }
    }

    fn register_worker(scheduler: &InMemoryScheduler) -> WorkerInfo {
        scheduler
            .register_worker(WorkerRegister {
                name: "local".to_string(),
                labels: HashMap::new(),
                capacity: WorkerCapacity::default(),
            })
            .expect("worker should register")
    }

    fn lease_single_task(scheduler: &InMemoryScheduler, worker_id: WorkerId) -> TaskLease {
        scheduler
            .submit_job(JobSubmit {
                name: "lease-demo".to_string(),
                priority: 0,
                max_retries: 0,
                tasks: vec![TaskSubmit {
                    name: "main".to_string(),
                    command: command("echo"),
                    dependencies: vec![],
                    requirements: Default::default(),
                    max_retries: None,
                    artifact_paths: Vec::new(),
                    openjd: None,
                }],
                openjd: None,
            })
            .expect("job should submit");
        scheduler
            .lease_task(worker_id)
            .expect("lease should work")
            .expect("a task should be leased")
    }

    fn temp_database_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("renderacre-{}.sqlite3", Uuid::new_v4()))
    }
}
