use std::net::SocketAddr;
use std::path::PathBuf;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Parser, ValueEnum};
use farm_core::{
    DashboardSnapshot, FarmError, FarmLogEntry, FarmStats, InMemoryScheduler, Job, JobId,
    JobPriorityUpdate, JobSubmit, SchedulerConfig, SqliteScheduler, Task, TaskComplete, TaskId,
    TaskLease, TaskLeaseRenewal, TaskStarted, WorkerId, WorkerInfo, WorkerLogBatch, WorkerRegister,
};
use serde_json::json;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "RFARM_BIND", default_value = "127.0.0.1:7878")]
    bind: SocketAddr,
    #[arg(long, env = "RFARM_LEASE_SECONDS", default_value_t = 120)]
    lease_seconds: i64,
    #[arg(long, env = "RFARM_STORAGE", value_enum, default_value_t = StorageBackend::Memory)]
    storage: StorageBackend,
    #[arg(long, env = "RFARM_SQLITE_PATH", default_value = "renderacre.sqlite3")]
    sqlite_path: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum StorageBackend {
    Memory,
    Sqlite,
}

#[derive(Clone)]
enum AppScheduler {
    Memory(InMemoryScheduler),
    Sqlite(SqliteScheduler),
}

impl AppScheduler {
    fn submit_job(&self, submission: JobSubmit) -> Result<Job, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.submit_job(submission),
            Self::Sqlite(scheduler) => scheduler.submit_job(submission),
        }
    }

    fn list_jobs(&self) -> Result<Vec<Job>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_jobs(),
            Self::Sqlite(scheduler) => scheduler.list_jobs(),
        }
    }

    fn get_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.get_job(job_id),
            Self::Sqlite(scheduler) => scheduler.get_job(job_id),
        }
    }

    fn list_workers(&self) -> Result<Vec<WorkerInfo>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_workers(),
            Self::Sqlite(scheduler) => scheduler.list_workers(),
        }
    }

    fn dashboard_snapshot(&self) -> Result<DashboardSnapshot, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.dashboard_snapshot(),
            Self::Sqlite(scheduler) => scheduler.dashboard_snapshot(),
        }
    }

    fn list_logs(&self) -> Result<Vec<FarmLogEntry>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_logs(),
            Self::Sqlite(scheduler) => scheduler.list_logs(),
        }
    }

    fn list_worker_logs(&self, worker_id: WorkerId) -> Result<Vec<FarmLogEntry>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_worker_logs(worker_id),
            Self::Sqlite(scheduler) => scheduler.list_worker_logs(worker_id),
        }
    }

    fn record_worker_logs(
        &self,
        worker_id: WorkerId,
        batch: WorkerLogBatch,
    ) -> Result<Vec<FarmLogEntry>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.record_worker_logs(worker_id, batch),
            Self::Sqlite(scheduler) => scheduler.record_worker_logs(worker_id, batch),
        }
    }

    fn pause_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.pause_job(job_id),
            Self::Sqlite(scheduler) => scheduler.pause_job(job_id),
        }
    }

    fn resume_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.resume_job(job_id),
            Self::Sqlite(scheduler) => scheduler.resume_job(job_id),
        }
    }

    fn cancel_job(&self, job_id: JobId) -> Result<Job, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.cancel_job(job_id),
            Self::Sqlite(scheduler) => scheduler.cancel_job(job_id),
        }
    }

    fn update_job_priority(&self, job_id: JobId, priority: i32) -> Result<Job, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.update_job_priority(job_id, priority),
            Self::Sqlite(scheduler) => scheduler.update_job_priority(job_id, priority),
        }
    }

    fn cancel_task(&self, task_id: TaskId) -> Result<Task, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.cancel_task(task_id),
            Self::Sqlite(scheduler) => scheduler.cancel_task(task_id),
        }
    }

    fn requeue_task(&self, task_id: TaskId) -> Result<Task, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.requeue_task(task_id),
            Self::Sqlite(scheduler) => scheduler.requeue_task(task_id),
        }
    }

    fn register_worker(&self, registration: WorkerRegister) -> Result<WorkerInfo, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.register_worker(registration),
            Self::Sqlite(scheduler) => scheduler.register_worker(registration),
        }
    }

    fn heartbeat_worker(&self, worker_id: WorkerId) -> Result<WorkerInfo, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.heartbeat_worker(worker_id),
            Self::Sqlite(scheduler) => scheduler.heartbeat_worker(worker_id),
        }
    }

    fn lease_task(&self, worker_id: WorkerId) -> Result<Option<TaskLease>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.lease_task(worker_id),
            Self::Sqlite(scheduler) => scheduler.lease_task(worker_id),
        }
    }

    fn mark_task_started(&self, task_id: TaskId, started: TaskStarted) -> Result<Task, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.mark_task_started(task_id, started),
            Self::Sqlite(scheduler) => scheduler.mark_task_started(task_id, started),
        }
    }

    fn complete_task(&self, task_id: TaskId, completion: TaskComplete) -> Result<Task, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.complete_task(task_id, completion),
            Self::Sqlite(scheduler) => scheduler.complete_task(task_id, completion),
        }
    }

    fn renew_task_lease(
        &self,
        task_id: TaskId,
        renewal: TaskLeaseRenewal,
    ) -> Result<Task, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.renew_task_lease(task_id, renewal),
            Self::Sqlite(scheduler) => scheduler.renew_task_lease(task_id, renewal),
        }
    }

    fn get_task_artifact(
        &self,
        task_id: TaskId,
        artifact_index: usize,
    ) -> Result<farm_core::TaskArtifact, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.get_task_artifact(task_id, artifact_index),
            Self::Sqlite(scheduler) => scheduler.get_task_artifact(task_id, artifact_index),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "farm_controller=info,tower_http=info".to_string()),
        )
        .init();

    let args = Args::parse();
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    let config = SchedulerConfig {
        lease_ttl_seconds: args.lease_seconds,
    };
    let scheduler = match args.storage {
        StorageBackend::Memory => AppScheduler::Memory(InMemoryScheduler::with_config(config)),
        StorageBackend::Sqlite => AppScheduler::Sqlite(SqliteScheduler::open_with_config(
            &args.sqlite_path,
            config,
        )?),
    };
    tracing::info!("controller listening on http://{}", args.bind);
    axum::serve(listener, app(scheduler)).await?;
    Ok(())
}

fn app(scheduler: AppScheduler) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/dashboard", get(get_dashboard))
        .route("/v1/logs", get(list_logs))
        .route("/v1/stats", get(get_stats))
        .route("/v1/jobs", get(list_jobs).post(submit_job))
        .route("/v1/jobs/{job_id}", get(get_job))
        .route("/v1/jobs/{job_id}/pause", post(pause_job))
        .route("/v1/jobs/{job_id}/resume", post(resume_job))
        .route("/v1/jobs/{job_id}/cancel", post(cancel_job))
        .route("/v1/jobs/{job_id}/priority", post(update_job_priority))
        .route("/v1/workers", get(list_workers))
        .route("/v1/workers/register", post(register_worker))
        .route(
            "/v1/workers/{worker_id}/logs",
            get(list_worker_logs).post(record_worker_logs),
        )
        .route("/v1/workers/{worker_id}/heartbeat", post(heartbeat_worker))
        .route("/v1/workers/{worker_id}/lease", post(lease_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
        .route("/v1/tasks/{task_id}/requeue", post(requeue_task))
        .route("/v1/tasks/{task_id}/started", post(mark_task_started))
        .route("/v1/tasks/{task_id}/renew", post(renew_task_lease))
        .route("/v1/tasks/{task_id}/complete", post(complete_task))
        .route(
            "/v1/tasks/{task_id}/artifacts/{artifact_index}",
            get(download_task_artifact),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(scheduler)
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

async fn submit_job(
    State(scheduler): State<AppScheduler>,
    Json(submission): Json<JobSubmit>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(scheduler.submit_job(submission)?))
}

async fn list_jobs(State(scheduler): State<AppScheduler>) -> Result<Json<Vec<Job>>, ApiError> {
    Ok(Json(scheduler.list_jobs()?))
}

async fn get_job(
    State(scheduler): State<AppScheduler>,
    Path(job_id): Path<JobId>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(scheduler.get_job(job_id)?))
}

async fn pause_job(
    State(scheduler): State<AppScheduler>,
    Path(job_id): Path<JobId>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(scheduler.pause_job(job_id)?))
}

async fn resume_job(
    State(scheduler): State<AppScheduler>,
    Path(job_id): Path<JobId>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(scheduler.resume_job(job_id)?))
}

async fn cancel_job(
    State(scheduler): State<AppScheduler>,
    Path(job_id): Path<JobId>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(scheduler.cancel_job(job_id)?))
}

async fn update_job_priority(
    State(scheduler): State<AppScheduler>,
    Path(job_id): Path<JobId>,
    Json(update): Json<JobPriorityUpdate>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(
        scheduler.update_job_priority(job_id, update.priority)?,
    ))
}

async fn list_workers(
    State(scheduler): State<AppScheduler>,
) -> Result<Json<Vec<WorkerInfo>>, ApiError> {
    Ok(Json(scheduler.list_workers()?))
}

async fn get_stats(State(scheduler): State<AppScheduler>) -> Result<Json<FarmStats>, ApiError> {
    Ok(Json(scheduler.dashboard_snapshot()?.stats))
}

async fn get_dashboard(
    State(scheduler): State<AppScheduler>,
) -> Result<Json<DashboardSnapshot>, ApiError> {
    Ok(Json(scheduler.dashboard_snapshot()?))
}

async fn list_logs(
    State(scheduler): State<AppScheduler>,
) -> Result<Json<Vec<FarmLogEntry>>, ApiError> {
    Ok(Json(scheduler.list_logs()?))
}

async fn register_worker(
    State(scheduler): State<AppScheduler>,
    Json(registration): Json<WorkerRegister>,
) -> Result<Json<WorkerInfo>, ApiError> {
    Ok(Json(scheduler.register_worker(registration)?))
}

async fn list_worker_logs(
    State(scheduler): State<AppScheduler>,
    Path(worker_id): Path<WorkerId>,
) -> Result<Json<Vec<FarmLogEntry>>, ApiError> {
    Ok(Json(scheduler.list_worker_logs(worker_id)?))
}

async fn record_worker_logs(
    State(scheduler): State<AppScheduler>,
    Path(worker_id): Path<WorkerId>,
    Json(batch): Json<WorkerLogBatch>,
) -> Result<Json<Vec<FarmLogEntry>>, ApiError> {
    Ok(Json(scheduler.record_worker_logs(worker_id, batch)?))
}

async fn heartbeat_worker(
    State(scheduler): State<AppScheduler>,
    Path(worker_id): Path<WorkerId>,
) -> Result<Json<WorkerInfo>, ApiError> {
    Ok(Json(scheduler.heartbeat_worker(worker_id)?))
}

async fn lease_task(
    State(scheduler): State<AppScheduler>,
    Path(worker_id): Path<WorkerId>,
) -> Result<Json<Option<TaskLease>>, ApiError> {
    Ok(Json(scheduler.lease_task(worker_id)?))
}

async fn cancel_task(
    State(scheduler): State<AppScheduler>,
    Path(task_id): Path<TaskId>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(scheduler.cancel_task(task_id)?))
}

async fn requeue_task(
    State(scheduler): State<AppScheduler>,
    Path(task_id): Path<TaskId>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(scheduler.requeue_task(task_id)?))
}

async fn mark_task_started(
    State(scheduler): State<AppScheduler>,
    Path(task_id): Path<TaskId>,
    Json(started): Json<TaskStarted>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(scheduler.mark_task_started(task_id, started)?))
}

async fn renew_task_lease(
    State(scheduler): State<AppScheduler>,
    Path(task_id): Path<TaskId>,
    Json(renewal): Json<TaskLeaseRenewal>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(scheduler.renew_task_lease(task_id, renewal)?))
}

async fn complete_task(
    State(scheduler): State<AppScheduler>,
    Path(task_id): Path<TaskId>,
    Json(completion): Json<TaskComplete>,
) -> Result<Json<Task>, ApiError> {
    Ok(Json(scheduler.complete_task(task_id, completion)?))
}

async fn download_task_artifact(
    State(scheduler): State<AppScheduler>,
    Path((task_id, artifact_index)): Path<(TaskId, usize)>,
) -> Result<Response, ApiError> {
    let artifact = scheduler.get_task_artifact(task_id, artifact_index)?;
    let bytes = tokio::fs::read(&artifact.path).await.map_err(|error| {
        ApiError::message(
            StatusCode::NOT_FOUND,
            format!("artifact file is not readable: {error}"),
        )
    })?;
    let filename = artifact.name.replace(['\\', '/', '"'], "_");
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type_for(&artifact.name)),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("inline; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("inline")),
    );
    Ok(response)
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn message(status: StatusCode, message: String) -> Self {
        Self { status, message }
    }
}

impl From<FarmError> for ApiError {
    fn from(value: FarmError) -> Self {
        let status = match value {
            FarmError::JobNotFound(_)
            | FarmError::TaskNotFound(_)
            | FarmError::WorkerNotFound(_)
            | FarmError::ArtifactNotFound { .. } => StatusCode::NOT_FOUND,
            FarmError::InvalidSubmission(_) => StatusCode::BAD_REQUEST,
            FarmError::InvalidLease | FarmError::InvalidState(_) => StatusCode::CONFLICT,
            FarmError::LockPoisoned | FarmError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": self.message,
        }));
        (self.status, body).into_response()
    }
}

fn content_type_for(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "txt" | "log" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "ma" | "mb" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
