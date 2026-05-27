use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use openjd_expr::value::Float64;
use openjd_expr::{ExprType, ExprValue, PathFormat, PathMappingRule, RangeExpr};
use openjd_model::{
    JobParameterType, JobParameterValue as ModelJobParameterValue, TaskParameterType,
    TaskParameterValue as ModelTaskParameterValue,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type JobId = Uuid;
pub type TaskId = Uuid;
pub type WorkerId = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSubmit {
    pub name: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub tasks: Vec<TaskSubmit>,
    #[serde(default)]
    pub openjd: Option<OpenJdSubmit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSubmit {
    pub name: String,
    pub command: CommandSpec,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub artifact_paths: Vec<PathBuf>,
    #[serde(default)]
    pub openjd: Option<OpenJdRuntimeTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenJdSubmit {
    pub template_yaml: String,
    #[serde(default)]
    pub parameters: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub asset_root: Option<PathBuf>,
    #[serde(default)]
    pub supported_extensions: Vec<String>,
    #[serde(default)]
    pub path_mapping_rules: Vec<OpenJdPathMappingRule>,
    #[serde(default)]
    pub template_dir: Option<PathBuf>,
    #[serde(default)]
    pub current_working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSpec {
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub working_dir: Option<PathBuf>,
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenJdPathMappingRule {
    pub source_path_format: OpenJdPathFormat,
    pub source_path: String,
    pub destination_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OpenJdPathFormat {
    Posix,
    Windows,
    Uri,
}

impl From<OpenJdPathFormat> for PathFormat {
    fn from(value: OpenJdPathFormat) -> Self {
        match value {
            OpenJdPathFormat::Posix => PathFormat::Posix,
            OpenJdPathFormat::Windows => PathFormat::Windows,
            OpenJdPathFormat::Uri => PathFormat::Uri,
        }
    }
}

impl From<PathFormat> for OpenJdPathFormat {
    fn from(value: PathFormat) -> Self {
        match value {
            PathFormat::Posix => OpenJdPathFormat::Posix,
            PathFormat::Windows => OpenJdPathFormat::Windows,
            PathFormat::Uri => OpenJdPathFormat::Uri,
        }
    }
}

impl From<OpenJdPathMappingRule> for PathMappingRule {
    fn from(value: OpenJdPathMappingRule) -> Self {
        Self {
            source_path_format: value.source_path_format.into(),
            source_path: value.source_path,
            destination_path: value.destination_path,
        }
    }
}

impl From<PathMappingRule> for OpenJdPathMappingRule {
    fn from(value: PathMappingRule) -> Self {
        Self {
            source_path_format: value.source_path_format.into(),
            source_path: value.source_path,
            destination_path: value.destination_path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenJdRuntimeTask {
    pub job_name: String,
    pub step_name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub job_parameters: HashMap<String, OpenJdParameterValue>,
    #[serde(default)]
    pub task_parameters: HashMap<String, OpenJdParameterValue>,
    #[serde(default)]
    pub path_mapping_rules: Vec<OpenJdPathMappingRule>,
    #[serde(default)]
    pub job_environments: Vec<OpenJdEnvironmentRuntime>,
    #[serde(default)]
    pub step_environments: Vec<serde_json::Value>,
    pub step_script: serde_json::Value,
    #[serde(default)]
    pub step_resolved_symtab: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenJdEnvironmentRuntime {
    pub environment: serde_json::Value,
    #[serde(default)]
    pub resolved_symtab: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenJdParameterValue {
    pub param_type: String,
    pub value: serde_json::Value,
}

impl OpenJdParameterValue {
    pub fn from_job_parameter(value: &ModelJobParameterValue) -> Self {
        Self {
            param_type: value.param_type.as_spec_str().to_string(),
            value: expr_value_to_json(&value.value),
        }
    }

    pub fn from_task_parameter(value: &ModelTaskParameterValue) -> Self {
        Self {
            param_type: value.param_type.as_spec_str().to_string(),
            value: expr_value_to_json(&value.value),
        }
    }

    pub fn to_job_parameter(&self) -> Result<ModelJobParameterValue, String> {
        let param_type = JobParameterType::from_spec_str(&self.param_type)
            .ok_or_else(|| format!("unknown OpenJD job parameter type '{}'", self.param_type))?;
        Ok(ModelJobParameterValue {
            param_type,
            value: json_to_expr_value(&self.value, &self.param_type)?,
        })
    }

    pub fn to_task_parameter(&self) -> Result<ModelTaskParameterValue, String> {
        let param_type = TaskParameterType::from_spec_str(&self.param_type)
            .ok_or_else(|| format!("unknown OpenJD task parameter type '{}'", self.param_type))?;
        Ok(ModelTaskParameterValue {
            param_type,
            value: json_to_expr_value(&self.value, &self.param_type)?,
        })
    }
}

pub fn json_to_openjd_input_value(value: &serde_json::Value) -> Result<ExprValue, String> {
    match value {
        serde_json::Value::Null => Ok(ExprValue::Null),
        serde_json::Value::Bool(value) => Ok(ExprValue::Bool(*value)),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                Ok(ExprValue::Int(value))
            } else {
                let value = value
                    .as_f64()
                    .ok_or_else(|| "number could not be represented as f64".to_string())?;
                Ok(ExprValue::Float(Float64::new(value).map_err(stringify)?))
            }
        }
        serde_json::Value::String(value) => Ok(ExprValue::String(value.clone())),
        serde_json::Value::Array(values) => {
            let values = values
                .iter()
                .map(json_to_openjd_input_value)
                .collect::<Result<Vec<_>, _>>()?;
            let elem_type = values
                .first()
                .map(|value| value.expr_type())
                .unwrap_or(ExprType::STRING);
            ExprValue::make_list(values, elem_type).map_err(stringify)
        }
        serde_json::Value::Object(_) => {
            Err("OpenJD parameter values must be scalar or list values".to_string())
        }
    }
}

fn expr_value_to_json(value: &ExprValue) -> serde_json::Value {
    match value {
        ExprValue::Null => serde_json::Value::Null,
        ExprValue::Bool(value) => serde_json::Value::Bool(*value),
        ExprValue::Int(value) => serde_json::json!(value),
        ExprValue::Float(value) => serde_json::json!(value.value()),
        ExprValue::String(value) => serde_json::json!(value),
        ExprValue::Path { value, .. } => serde_json::json!(value),
        ExprValue::ListBool(values) => serde_json::json!(values),
        ExprValue::ListInt(values) => serde_json::json!(values),
        ExprValue::ListFloat(values) => serde_json::Value::Array(
            values
                .iter()
                .map(|value| serde_json::json!(value.value()))
                .collect(),
        ),
        ExprValue::ListString(values, _) | ExprValue::ListPath(values, _, _) => {
            serde_json::json!(values)
        }
        ExprValue::ListList(values, _, _) => {
            serde_json::Value::Array(values.iter().map(expr_value_to_json).collect())
        }
        ExprValue::RangeExpr(value) => serde_json::json!(value.to_string()),
        ExprValue::Unresolved(value_type) => serde_json::json!(format!("{value_type:?}")),
        _ => serde_json::json!(format!("{value:?}")),
    }
}

fn json_to_expr_value(value: &serde_json::Value, param_type: &str) -> Result<ExprValue, String> {
    match param_type.to_ascii_uppercase().as_str() {
        "STRING" => Ok(ExprValue::String(value_as_string(value)?)),
        "INT" => Ok(ExprValue::Int(value_as_i64(value)?)),
        "FLOAT" => Ok(ExprValue::Float(
            Float64::new(value_as_f64(value)?).map_err(stringify)?,
        )),
        "BOOL" => Ok(ExprValue::Bool(value_as_bool(value)?)),
        "PATH" => Ok(ExprValue::new_path(
            value_as_string(value)?,
            PathFormat::host(),
        )),
        "RANGE_EXPR" | "CHUNK[INT]" => Ok(ExprValue::RangeExpr(
            value_as_string(value)?
                .parse::<RangeExpr>()
                .map_err(stringify)?,
        )),
        "LIST[STRING]" => list_from_json(
            value,
            |value| Ok(ExprValue::String(value_as_string(value)?)),
            ExprType::STRING,
        ),
        "LIST[INT]" => list_from_json(
            value,
            |value| Ok(ExprValue::Int(value_as_i64(value)?)),
            ExprType::INT,
        ),
        "LIST[FLOAT]" => list_from_json(
            value,
            |value| {
                Ok(ExprValue::Float(
                    Float64::new(value_as_f64(value)?).map_err(stringify)?,
                ))
            },
            ExprType::FLOAT,
        ),
        "LIST[BOOL]" => list_from_json(
            value,
            |value| Ok(ExprValue::Bool(value_as_bool(value)?)),
            ExprType::BOOL,
        ),
        "LIST[PATH]" => list_from_json(
            value,
            |value| {
                Ok(ExprValue::new_path(
                    value_as_string(value)?,
                    PathFormat::host(),
                ))
            },
            ExprType::PATH,
        ),
        "LIST[LIST[INT]]" => list_from_json(
            value,
            |value| {
                list_from_json(
                    value,
                    |inner| Ok(ExprValue::Int(value_as_i64(inner)?)),
                    ExprType::INT,
                )
            },
            ExprType::list(ExprType::INT),
        ),
        other => Err(format!("unsupported OpenJD parameter type '{other}'")),
    }
}

fn list_from_json<F>(
    value: &serde_json::Value,
    mut convert: F,
    elem_type: ExprType,
) -> Result<ExprValue, String>
where
    F: FnMut(&serde_json::Value) -> Result<ExprValue, String>,
{
    let values = value
        .as_array()
        .ok_or_else(|| "expected JSON array for OpenJD list parameter".to_string())?
        .iter()
        .map(&mut convert)
        .collect::<Result<Vec<_>, _>>()?;
    ExprValue::make_list(values, elem_type).map_err(stringify)
}

fn value_as_string(value: &serde_json::Value) -> Result<String, String> {
    match value {
        serde_json::Value::String(value) => Ok(value.clone()),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => Ok(value.to_string()),
        _ => Err("expected scalar string-compatible OpenJD value".to_string()),
    }
}

fn value_as_i64(value: &serde_json::Value) -> Result<i64, String> {
    match value {
        serde_json::Value::Number(value) => value
            .as_i64()
            .ok_or_else(|| "expected integer OpenJD value".to_string()),
        serde_json::Value::String(value) => value.parse::<i64>().map_err(stringify),
        _ => Err("expected integer OpenJD value".to_string()),
    }
}

fn value_as_f64(value: &serde_json::Value) -> Result<f64, String> {
    match value {
        serde_json::Value::Number(value) => value
            .as_f64()
            .ok_or_else(|| "expected float OpenJD value".to_string()),
        serde_json::Value::String(value) => value.parse::<f64>().map_err(stringify),
        _ => Err("expected float OpenJD value".to_string()),
    }
}

fn value_as_bool(value: &serde_json::Value) -> Result<bool, String> {
    match value {
        serde_json::Value::Bool(value) => Ok(*value),
        serde_json::Value::String(value) => value.parse::<bool>().map_err(stringify),
        _ => Err("expected boolean OpenJD value".to_string()),
    }
}

fn stringify<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub name: String,
    pub state: JobState,
    pub priority: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub openjd: Option<OpenJdSubmit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub stats: FarmStats,
    pub jobs: Vec<Job>,
    pub workers: Vec<WorkerInfo>,
    pub logs: Vec<FarmLogEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FarmStats {
    pub jobs_total: usize,
    pub jobs_queued: usize,
    pub jobs_running: usize,
    pub jobs_succeeded: usize,
    pub jobs_failed: usize,
    pub tasks_total: usize,
    pub tasks_pending: usize,
    pub tasks_leased: usize,
    pub tasks_running: usize,
    pub tasks_succeeded: usize,
    pub tasks_failed: usize,
    pub workers_total: usize,
    pub workers_online: usize,
    pub workers_offline: usize,
    pub worker_slots: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub job_id: JobId,
    pub name: String,
    pub state: TaskState,
    pub command: CommandSpec,
    pub dependencies: Vec<TaskId>,
    pub attempts: u32,
    pub max_retries: u32,
    pub lease: Option<TaskLeaseInfo>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub last_exit_code: Option<i32>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
    #[serde(default)]
    pub artifact_paths: Vec<PathBuf>,
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
    #[serde(default)]
    pub openjd: Option<OpenJdRuntimeTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskArtifact {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub kind: ArtifactKind,
    #[serde(default)]
    pub modified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Image,
    Scene,
    Log,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Leased,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegister {
    pub name: String,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub capacity: WorkerCapacity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerCapacity {
    pub slots: u32,
}

impl Default for WorkerCapacity {
    fn default() -> Self {
        Self { slots: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub id: WorkerId,
    pub name: String,
    pub labels: HashMap<String, String>,
    pub capacity: WorkerCapacity,
    pub state: WorkerState,
    pub registered_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLogBatch {
    #[serde(default)]
    pub entries: Vec<WorkerLogInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLogInput {
    pub level: LogLevel,
    #[serde(default)]
    pub stream: Option<String>,
    pub message: String,
    #[serde(default)]
    pub job_id: Option<JobId>,
    #[serde(default)]
    pub task_id: Option<TaskId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmLogEntry {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub source: LogSource,
    #[serde(default)]
    pub stream: Option<String>,
    pub message: String,
    #[serde(default)]
    pub job_id: Option<JobId>,
    #[serde(default)]
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub worker_id: Option<WorkerId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogSource {
    Controller,
    Worker,
    Task,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLeaseInfo {
    pub token: String,
    pub worker_id: WorkerId,
    pub leased_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLease {
    pub task: Task,
    pub lease_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStarted {
    pub worker_id: WorkerId,
    pub lease_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLeaseRenewal {
    pub worker_id: WorkerId,
    pub lease_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskComplete {
    pub worker_id: WorkerId,
    pub lease_token: String,
    pub exit_code: i32,
    #[serde(default)]
    pub stdout_tail: Option<String>,
    #[serde(default)]
    pub stderr_tail: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<TaskArtifact>,
}

fn default_max_retries() -> u32 {
    0
}
