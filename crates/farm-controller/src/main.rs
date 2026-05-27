use std::net::SocketAddr;
use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::{Parser, ValueEnum};
use farm_core::{
    DashboardSnapshot, FarmError, FarmStats, InMemoryScheduler, Job, JobId, JobSubmit,
    SchedulerConfig, SqliteScheduler, Task, TaskComplete, TaskId, TaskLease, TaskLeaseRenewal,
    TaskStarted, WorkerId, WorkerInfo, WorkerRegister,
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
        .route("/v1/stats", get(get_stats))
        .route("/v1/jobs", get(list_jobs).post(submit_job))
        .route("/v1/jobs/{job_id}", get(get_job))
        .route("/v1/workers", get(list_workers))
        .route("/v1/workers/register", post(register_worker))
        .route("/v1/workers/{worker_id}/heartbeat", post(heartbeat_worker))
        .route("/v1/workers/{worker_id}/lease", post(lease_task))
        .route("/v1/tasks/{task_id}/started", post(mark_task_started))
        .route("/v1/tasks/{task_id}/renew", post(renew_task_lease))
        .route("/v1/tasks/{task_id}/complete", post(complete_task))
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

async fn register_worker(
    State(scheduler): State<AppScheduler>,
    Json(registration): Json<WorkerRegister>,
) -> Result<Json<WorkerInfo>, ApiError> {
    Ok(Json(scheduler.register_worker(registration)?))
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

struct ApiError(FarmError);

impl From<FarmError> for ApiError {
    fn from(value: FarmError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            FarmError::JobNotFound(_)
            | FarmError::TaskNotFound(_)
            | FarmError::WorkerNotFound(_) => StatusCode::NOT_FOUND,
            FarmError::InvalidSubmission(_) => StatusCode::BAD_REQUEST,
            FarmError::InvalidLease => StatusCode::CONFLICT,
            FarmError::LockPoisoned | FarmError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(json!({
            "error": self.0.to_string(),
        }));
        (status, body).into_response()
    }
}
