#[cfg(test)]
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};

use anyhow::Context;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, ValueEnum};
use farm_core::{
    AuditEvent, DashboardSnapshot, FarmError, FarmLogEntry, FarmMetrics, FarmStats,
    HealthComponent, HealthReport, HealthStatus, InMemoryScheduler, Job, JobId, JobPriorityUpdate,
    JobSubmit, ResourceLimitDefinition, ResourceLimitSnapshot, SchedulerConfig, SqliteScheduler,
    Task, TaskComplete, TaskId, TaskLease, TaskLeaseRenewal, TaskStarted, WorkerId, WorkerInfo,
    WorkerLogBatch, WorkerRegister,
};
use serde::Deserialize;
use serde_json::json;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "RFARM_CONFIG")]
    config: Option<PathBuf>,
    #[arg(long, env = "RFARM_BIND", default_value = "127.0.0.1:7878")]
    bind: SocketAddr,
    #[arg(long, env = "RFARM_LEASE_SECONDS", default_value_t = 120)]
    lease_seconds: i64,
    #[arg(long, env = "RFARM_STORAGE", value_enum, default_value_t = StorageBackend::Memory)]
    storage: StorageBackend,
    #[arg(long, env = "RFARM_SQLITE_PATH", default_value = "renderacre.sqlite3")]
    sqlite_path: PathBuf,
    #[arg(long, env = "RFARM_DASHBOARD_DIR")]
    dashboard_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StorageBackend {
    Memory,
    Sqlite,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ControllerConfig {
    bind: Option<SocketAddr>,
    lease_seconds: Option<i64>,
    storage: Option<StorageBackend>,
    sqlite_path: Option<PathBuf>,
    dashboard_dir: Option<PathBuf>,
}

impl Args {
    fn load() -> anyhow::Result<Self> {
        Self::from_matches(Self::command().get_matches())
    }

    #[cfg(test)]
    fn load_from<I, T>(iter: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = Self::command().try_get_matches_from(iter)?;
        Self::from_matches(matches)
    }

    fn from_matches(matches: ArgMatches) -> anyhow::Result<Self> {
        let mut args = Self::from_arg_matches(&matches)?;
        let config = ControllerConfig::load(args.config.as_deref())?;
        args.apply_config(config, &matches);
        Ok(args)
    }

    fn apply_config(&mut self, config: ControllerConfig, matches: &ArgMatches) {
        if value_from_default(matches, "bind") {
            if let Some(bind) = config.bind {
                self.bind = bind;
            }
        }
        if value_from_default(matches, "lease_seconds") {
            if let Some(lease_seconds) = config.lease_seconds {
                self.lease_seconds = lease_seconds;
            }
        }
        if value_from_default(matches, "storage") {
            if let Some(storage) = config.storage {
                self.storage = storage;
            }
        }
        if value_from_default(matches, "sqlite_path") {
            if let Some(sqlite_path) = config.sqlite_path {
                self.sqlite_path = sqlite_path;
            }
        }
        if matches.value_source("dashboard_dir").is_none() {
            if let Some(dashboard_dir) = config.dashboard_dir {
                self.dashboard_dir = Some(dashboard_dir);
            }
        }
    }
}

impl ControllerConfig {
    fn load(path: Option<&FsPath>) -> anyhow::Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse config file {}", path.display()))
    }
}

fn value_from_default(matches: &ArgMatches, id: &str) -> bool {
    matches.value_source(id) == Some(ValueSource::DefaultValue)
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

    fn metrics_snapshot(&self) -> Result<FarmMetrics, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.metrics_snapshot(),
            Self::Sqlite(scheduler) => scheduler.metrics_snapshot(),
        }
    }

    fn list_audit_events(&self) -> Result<Vec<AuditEvent>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_audit_events(),
            Self::Sqlite(scheduler) => scheduler.list_audit_events(),
        }
    }

    fn backend_name(&self) -> &'static str {
        match self {
            Self::Memory(_) => "memory",
            Self::Sqlite(_) => "sqlite",
        }
    }

    fn health_report(&self) -> HealthReport {
        match self.metrics_snapshot() {
            Ok(_) => HealthReport {
                status: HealthStatus::Ready,
                controller: HealthComponent {
                    status: HealthStatus::Ready,
                    backend: None,
                    message: Some("controller ready".to_string()),
                },
                scheduler: HealthComponent {
                    status: HealthStatus::Ready,
                    backend: Some(self.backend_name().to_string()),
                    message: Some("scheduler ready".to_string()),
                },
                degraded: Vec::new(),
            },
            Err(error) => HealthReport {
                status: HealthStatus::Degraded,
                controller: HealthComponent {
                    status: HealthStatus::Ready,
                    backend: None,
                    message: Some("controller ready".to_string()),
                },
                scheduler: HealthComponent {
                    status: HealthStatus::Degraded,
                    backend: Some(self.backend_name().to_string()),
                    message: Some(error.to_string()),
                },
                degraded: vec!["scheduler".to_string()],
            },
        }
    }

    fn list_worker_logs(&self, worker_id: WorkerId) -> Result<Vec<FarmLogEntry>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_worker_logs(worker_id),
            Self::Sqlite(scheduler) => scheduler.list_worker_logs(worker_id),
        }
    }

    fn list_task_attempt_logs(
        &self,
        task_id: TaskId,
        attempt_id: uuid::Uuid,
    ) -> Result<Vec<FarmLogEntry>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_task_attempt_logs(task_id, attempt_id),
            Self::Sqlite(scheduler) => scheduler.list_task_attempt_logs(task_id, attempt_id),
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

    fn define_limit(
        &self,
        definition: ResourceLimitDefinition,
    ) -> Result<ResourceLimitSnapshot, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.define_limit(definition),
            Self::Sqlite(scheduler) => scheduler.define_limit(definition),
        }
    }

    fn list_limits(&self) -> Result<Vec<ResourceLimitSnapshot>, FarmError> {
        match self {
            Self::Memory(scheduler) => scheduler.list_limits(),
            Self::Sqlite(scheduler) => scheduler.list_limits(),
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

    let args = Args::load()?;
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
    if let Some(dashboard_dir) = args.dashboard_dir.as_ref() {
        tracing::info!(path = %dashboard_dir.display(), "serving dashboard assets");
    }
    axum::serve(listener, app(scheduler, args.dashboard_dir)).await?;
    Ok(())
}

fn app(scheduler: AppScheduler, dashboard_dir: Option<PathBuf>) -> Router {
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(healthz))
        .route("/v1/health", get(healthz))
        .route("/v1/dashboard", get(get_dashboard))
        .route("/v1/logs", get(list_logs))
        .route("/v1/audit", get(list_audit_events))
        .route("/v1/metrics", get(get_metrics))
        .route("/v1/stats", get(get_stats))
        .route("/v1/limits", get(list_limits).post(define_limit))
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
            "/v1/tasks/{task_id}/attempts/{attempt_id}/logs",
            get(list_task_attempt_logs),
        )
        .route(
            "/v1/tasks/{task_id}/artifacts/{artifact_index}",
            get(download_task_artifact),
        )
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(scheduler);

    if let Some(dashboard_dir) = dashboard_dir {
        let index = dashboard_dir.join("index.html");
        router.fallback_service(ServeDir::new(dashboard_dir).fallback(ServeFile::new(index)))
    } else {
        router
    }
}

async fn healthz(State(scheduler): State<AppScheduler>) -> Json<HealthReport> {
    Json(scheduler.health_report())
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

async fn get_metrics(State(scheduler): State<AppScheduler>) -> Result<Json<FarmMetrics>, ApiError> {
    Ok(Json(scheduler.metrics_snapshot()?))
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

async fn list_audit_events(
    State(scheduler): State<AppScheduler>,
) -> Result<Json<Vec<AuditEvent>>, ApiError> {
    Ok(Json(scheduler.list_audit_events()?))
}

async fn define_limit(
    State(scheduler): State<AppScheduler>,
    Json(definition): Json<ResourceLimitDefinition>,
) -> Result<Json<ResourceLimitSnapshot>, ApiError> {
    Ok(Json(scheduler.define_limit(definition)?))
}

async fn list_limits(
    State(scheduler): State<AppScheduler>,
) -> Result<Json<Vec<ResourceLimitSnapshot>>, ApiError> {
    Ok(Json(scheduler.list_limits()?))
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

async fn list_task_attempt_logs(
    State(scheduler): State<AppScheduler>,
    Path((task_id, attempt_id)): Path<(TaskId, uuid::Uuid)>,
) -> Result<Json<Vec<FarmLogEntry>>, ApiError> {
    Ok(Json(scheduler.list_task_attempt_logs(task_id, attempt_id)?))
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
            FarmError::AttemptNotFound { .. } => StatusCode::NOT_FOUND,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_overrides_controller_defaults() {
        let config_path = write_config(
            "controller",
            r#"
bind: 127.0.0.1:9001
lease_seconds: 45
storage: sqlite
sqlite_path: data/renderacre.sqlite3
dashboard_dir: dashboard/dist
"#,
        );

        let args = Args::load_from([
            "renderacre-controller",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .expect("config should load");

        assert_eq!(args.bind, "127.0.0.1:9001".parse::<SocketAddr>().unwrap());
        assert_eq!(args.lease_seconds, 45);
        assert_eq!(args.storage, StorageBackend::Sqlite);
        assert_eq!(args.sqlite_path, PathBuf::from("data/renderacre.sqlite3"));
        assert_eq!(args.dashboard_dir, Some(PathBuf::from("dashboard/dist")));
        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn command_line_values_override_controller_config() {
        let config_path = write_config(
            "controller-cli",
            r#"
bind: 127.0.0.1:9001
lease_seconds: 45
storage: sqlite
sqlite_path: data/renderacre.sqlite3
dashboard_dir: dashboard/dist
"#,
        );

        let args = Args::load_from([
            "renderacre-controller",
            "--config",
            config_path.to_str().unwrap(),
            "--bind",
            "127.0.0.1:9002",
            "--storage",
            "memory",
        ])
        .expect("config should load");

        assert_eq!(args.bind, "127.0.0.1:9002".parse::<SocketAddr>().unwrap());
        assert_eq!(args.storage, StorageBackend::Memory);
        assert_eq!(args.lease_seconds, 45);
        assert_eq!(args.dashboard_dir, Some(PathBuf::from("dashboard/dist")));
        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn health_report_includes_controller_and_scheduler_status() {
        let scheduler = AppScheduler::Memory(InMemoryScheduler::default());
        let report = scheduler.health_report();

        assert_eq!(report.status, HealthStatus::Ready);
        assert_eq!(report.controller.status, HealthStatus::Ready);
        assert_eq!(report.scheduler.status, HealthStatus::Ready);
        assert_eq!(report.scheduler.backend.as_deref(), Some("memory"));
        assert!(report.degraded.is_empty());
    }

    fn write_config(name: &str, content: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("renderacre-{name}-{}.yaml", uuid::Uuid::new_v4()));
        std::fs::write(&path, content).expect("config should write");
        path
    }
}
