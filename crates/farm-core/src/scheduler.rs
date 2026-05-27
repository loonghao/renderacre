use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::models::{
    Job, JobId, JobState, JobSubmit, Task, TaskComplete, TaskId, TaskLease, TaskLeaseInfo,
    TaskStarted, TaskState, TaskSubmit, WorkerId, WorkerInfo, WorkerRegister, WorkerState,
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
    #[error("invalid submission: {0}")]
    InvalidSubmission(String),
    #[error("lease is invalid or expired")]
    InvalidLease,
    #[error("scheduler lock was poisoned")]
    LockPoisoned,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryScheduler {
    inner: Arc<Mutex<SchedulerState>>,
}

#[derive(Debug, Default)]
struct SchedulerState {
    jobs: HashMap<JobId, Job>,
    workers: HashMap<WorkerId, WorkerInfo>,
}

impl InMemoryScheduler {
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

    pub fn register_worker(&self, registration: WorkerRegister) -> Result<WorkerInfo, FarmError> {
        let now = Utc::now();
        let worker = WorkerInfo {
            id: Uuid::new_v4(),
            name: registration.name,
            labels: registration.labels,
            capacity: registration.capacity,
            state: WorkerState::Online,
            registered_at: now,
            last_seen_at: now,
        };
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        state.workers.insert(worker.id, worker.clone());
        Ok(worker)
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

    pub fn lease_task(&self, worker_id: WorkerId) -> Result<Option<TaskLease>, FarmError> {
        let mut state = self.inner.lock().map_err(|_| FarmError::LockPoisoned)?;
        if !state.workers.contains_key(&worker_id) {
            return Err(FarmError::WorkerNotFound(worker_id));
        }

        expire_old_leases(&mut state);

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
                expires_at: now + Duration::seconds(120),
            });
            task.clone()
        };
        update_job_state(job);

        Ok(Some(TaskLease {
            task: leased_task,
            lease_token,
        }))
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
        Ok(task)
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

        task.completed_at = Some(Utc::now());
        task.updated_at = Utc::now();
        task.last_exit_code = Some(completion.exit_code);
        task.stdout_tail = completion.stdout_tail;
        task.stderr_tail = completion.stderr_tail;
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
        Ok(task)
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
        attempts: 0,
        max_retries: submission.max_retries.unwrap_or(job_max_retries),
        lease: None,
        created_at: now,
        updated_at: now,
        started_at: None,
        completed_at: None,
        last_exit_code: None,
        stdout_tail: None,
        stderr_tail: None,
    })
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

#[cfg(test)]
mod tests {
    use crate::models::{CommandSpec, WorkerCapacity};

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
                        max_retries: None,
                        openjd: None,
                    },
                    TaskSubmit {
                        name: "render".to_string(),
                        command: command("echo"),
                        dependencies: vec!["prepare".to_string()],
                        max_retries: None,
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

    fn command(executable: &str) -> CommandSpec {
        CommandSpec {
            executable: executable.to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
            timeout_seconds: None,
        }
    }
}
