use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser};
use farm_core::{
    ArtifactKind, LogLevel, OpenJdRuntimeTask, Task, TaskArtifact, TaskComplete, TaskLease,
    TaskLeaseRenewal, TaskStarted, WorkerCapacity, WorkerId, WorkerInfo, WorkerLogBatch,
    WorkerLogInput, WorkerRegister,
};
use futures_util::future::{FutureExt, LocalBoxFuture};
use futures_util::stream::{FuturesUnordered, StreamExt};
use openjd_expr::SerializedSymbolTable;
use openjd_model::{ModelExtension, ModelProfile, SpecificationRevision, TaskParameterSet};
use openjd_sessions::{ActionState, ActionStatus, Session, SessionConfig, StickyBitPolicy};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "RFARM_CONFIG")]
    config: Option<PathBuf>,
    #[arg(
        long,
        env = "RFARM_CONTROLLER",
        default_value = "http://127.0.0.1:7878"
    )]
    controller: String,
    #[arg(long, env = "RFARM_WORKER_NAME")]
    name: Option<String>,
    #[arg(
        long = "label",
        env = "RFARM_WORKER_LABELS",
        value_delimiter = ',',
        value_parser = parse_label
    )]
    labels: Vec<(String, String)>,
    #[arg(long, env = "RFARM_WORKER_SLOTS", default_value_t = 1)]
    slots: u32,
    #[arg(long, env = "RFARM_WORKER_POLL_SECONDS", default_value_t = 2)]
    poll_seconds: u64,
    #[arg(long, env = "RFARM_LEASE_RENEW_SECONDS", default_value_t = 30)]
    lease_renew_seconds: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct WorkerConfig {
    controller: Option<String>,
    name: Option<String>,
    labels: HashMap<String, String>,
    slots: Option<u32>,
    poll_seconds: Option<u64>,
    lease_renew_seconds: Option<u64>,
}

impl Args {
    fn load() -> Result<Self> {
        Self::from_matches(Self::command().get_matches())
    }

    #[cfg(test)]
    fn load_from<I, T>(iter: I) -> Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let matches = Self::command().try_get_matches_from(iter)?;
        Self::from_matches(matches)
    }

    fn from_matches(matches: ArgMatches) -> Result<Self> {
        let mut args = Self::from_arg_matches(&matches)?;
        let config = WorkerConfig::load(args.config.as_deref())?;
        args.apply_config(config, &matches);
        Ok(args)
    }

    fn apply_config(&mut self, config: WorkerConfig, matches: &ArgMatches) {
        if value_from_default(matches, "controller") {
            if let Some(controller) = config.controller {
                self.controller = controller;
            }
        }
        if matches.value_source("name").is_none() {
            if let Some(name) = config.name {
                self.name = Some(name);
            }
        }
        if matches.value_source("labels").is_none() && !config.labels.is_empty() {
            self.labels = config.labels.into_iter().collect();
            self.labels
                .sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
        }
        if value_from_default(matches, "slots") {
            if let Some(slots) = config.slots {
                self.slots = slots;
            }
        }
        if value_from_default(matches, "poll_seconds") {
            if let Some(poll_seconds) = config.poll_seconds {
                self.poll_seconds = poll_seconds;
            }
        }
        if value_from_default(matches, "lease_renew_seconds") {
            if let Some(lease_renew_seconds) = config.lease_renew_seconds {
                self.lease_renew_seconds = lease_renew_seconds;
            }
        }
    }
}

impl WorkerConfig {
    fn load(path: Option<&Path>) -> Result<Self> {
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
struct WorkerLogSink {
    client: reqwest::Client,
    controller: String,
    worker_id: WorkerId,
}

type OpenJdStatusCallback = Box<dyn Fn(&str, &ActionStatus) + Send + Sync>;

impl WorkerLogSink {
    fn new(client: reqwest::Client, controller: String, worker_id: WorkerId) -> Self {
        Self {
            client,
            controller,
            worker_id,
        }
    }

    async fn post_line(
        &self,
        level: LogLevel,
        stream: Option<String>,
        message: String,
        job_id: Option<uuid::Uuid>,
        task_id: Option<uuid::Uuid>,
    ) {
        self.post_batch(vec![WorkerLogInput {
            level,
            stream,
            message,
            job_id,
            task_id,
            attempt_id: None,
        }])
        .await;
    }

    async fn post_lines(
        &self,
        job_id: uuid::Uuid,
        task_id: uuid::Uuid,
        stream_name: &str,
        level: LogLevel,
        buffer: &str,
    ) {
        let entries = buffer
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| WorkerLogInput {
                level: level.clone(),
                stream: Some(stream_name.to_string()),
                message: line.to_string(),
                job_id: Some(job_id),
                task_id: Some(task_id),
                attempt_id: None,
            })
            .collect::<Vec<_>>();
        self.post_batch(entries).await;
    }

    async fn post_batch(&self, entries: Vec<WorkerLogInput>) {
        if entries.is_empty() {
            return;
        }
        let _ = self
            .client
            .post(format!(
                "{}/v1/workers/{}/logs",
                self.controller, self.worker_id
            ))
            .json(&WorkerLogBatch { entries })
            .send()
            .await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "farm_worker=info".to_string()),
        )
        .init();

    let args = Args::load()?;
    let client = reqwest::Client::new();
    let controller = args.controller.trim_end_matches('/').to_string();
    let worker = register_worker(&client, &controller, &args).await?;
    tracing::info!(worker_id = %worker.id, "worker registered");
    let log_sink = WorkerLogSink::new(client.clone(), controller.clone(), worker.id);
    log_sink
        .post_line(
            LogLevel::Info,
            None,
            "worker online".to_string(),
            None,
            None,
        )
        .await;
    let slots = worker.capacity.slots.max(1) as usize;
    let lease_renew_seconds = args.lease_renew_seconds.max(1);
    let mut running = FuturesUnordered::<LocalBoxFuture<'static, Result<()>>>::new();

    loop {
        heartbeat(&client, &controller, worker.id).await?;
        while running.len() < slots {
            let Some(lease) = lease_task(&client, &controller, worker.id).await? else {
                break;
            };
            let client = client.clone();
            let controller = controller.clone();
            let log_sink = log_sink.clone();
            running.push(
                async move {
                    run_lease(&client, &controller, &log_sink, lease, lease_renew_seconds).await
                }
                .boxed_local(),
            );
        }

        if running.is_empty() {
            tokio::time::sleep(Duration::from_secs(args.poll_seconds)).await;
        } else {
            tokio::select! {
                result = running.next() => {
                    if let Some(result) = result {
                        result?;
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(args.poll_seconds)) => {}
            }
        }
    }
}

async fn register_worker(
    client: &reqwest::Client,
    controller: &str,
    args: &Args,
) -> Result<WorkerInfo> {
    let mut labels = args.labels.iter().cloned().collect::<HashMap<_, _>>();
    add_default_openjd_capabilities(&mut labels, args.slots.max(1));
    let name = args.name.clone().unwrap_or_else(default_worker_name);
    let worker = client
        .post(format!("{controller}/v1/workers/register"))
        .json(&WorkerRegister {
            name,
            labels,
            capacity: WorkerCapacity {
                slots: args.slots.max(1),
            },
            identity: None,
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(worker)
}

fn add_default_openjd_capabilities(labels: &mut HashMap<String, String>, slots: u32) {
    labels
        .entry("attr.worker.os.family".to_string())
        .or_insert_with(openjd_os_family);
    labels
        .entry("attr.worker.cpu.arch".to_string())
        .or_insert_with(openjd_cpu_arch);
    labels
        .entry("amount.worker.vcpu".to_string())
        .or_insert_with(|| slots.max(1).to_string());
}

fn openjd_os_family() -> String {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        _ => "linux",
    }
    .to_string()
}

fn openjd_cpu_arch() -> String {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => other,
    }
    .to_string()
}

async fn heartbeat(client: &reqwest::Client, controller: &str, worker_id: WorkerId) -> Result<()> {
    client
        .post(format!("{controller}/v1/workers/{worker_id}/heartbeat"))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn lease_task(
    client: &reqwest::Client,
    controller: &str,
    worker_id: WorkerId,
) -> Result<Option<TaskLease>> {
    let lease = client
        .post(format!("{controller}/v1/workers/{worker_id}/lease"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(lease)
}

async fn run_lease(
    client: &reqwest::Client,
    controller: &str,
    log_sink: &WorkerLogSink,
    lease: TaskLease,
    lease_renew_seconds: u64,
) -> Result<()> {
    tracing::info!(task_id = %lease.task.id, task = %lease.task.name, "running task");
    log_sink
        .post_line(
            LogLevel::Info,
            None,
            format!("starting task {}", lease.task.name),
            Some(lease.task.job_id),
            Some(lease.task.id),
        )
        .await;
    client
        .post(format!("{controller}/v1/tasks/{}/started", lease.task.id))
        .json(&TaskStarted {
            worker_id: log_sink.worker_id,
            lease_token: lease.lease_token.clone(),
        })
        .send()
        .await?
        .error_for_status()?;

    let output =
        execute_with_lease_renewal(log_sink, controller, &lease, lease_renew_seconds).await;
    let completion = match output {
        Ok(output) => TaskComplete {
            worker_id: log_sink.worker_id,
            lease_token: lease.lease_token,
            exit_code: output.exit_code,
            stdout_tail: Some(tail_text(output.stdout.as_bytes(), 8192)),
            stderr_tail: Some(tail_text(output.stderr.as_bytes(), 8192)),
            artifacts: output.artifacts,
        },
        Err(err) => {
            let message = err.to_string();
            log_sink
                .post_line(
                    LogLevel::Error,
                    Some("worker".to_string()),
                    message.clone(),
                    Some(lease.task.job_id),
                    Some(lease.task.id),
                )
                .await;
            TaskComplete {
                worker_id: log_sink.worker_id,
                lease_token: lease.lease_token,
                exit_code: -1,
                stdout_tail: None,
                stderr_tail: Some(message),
                artifacts: Vec::new(),
            }
        }
    };

    client
        .post(format!("{controller}/v1/tasks/{}/complete", lease.task.id))
        .json(&completion)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn execute_with_lease_renewal(
    log_sink: &WorkerLogSink,
    controller: &str,
    lease: &TaskLease,
    lease_renew_seconds: u64,
) -> Result<ExecutionOutput> {
    let mut execution = Box::pin(execute_task(log_sink, lease));
    let mut renew_timer = tokio::time::interval(Duration::from_secs(lease_renew_seconds.max(1)));
    renew_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renew_timer.tick().await;

    loop {
        tokio::select! {
            output = &mut execution => return output,
            _ = renew_timer.tick() => {
                if let Err(error) = renew_task_lease(&log_sink.client, controller, log_sink.worker_id, lease).await {
                    tracing::warn!(task_id = %lease.task.id, error = %error, "failed to renew task lease");
                }
            }
        }
    }
}

async fn renew_task_lease(
    client: &reqwest::Client,
    controller: &str,
    worker_id: WorkerId,
    lease: &TaskLease,
) -> Result<()> {
    client
        .post(format!("{controller}/v1/tasks/{}/renew", lease.task.id))
        .json(&TaskLeaseRenewal {
            worker_id,
            lease_token: lease.lease_token.clone(),
        })
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

struct ExecutionOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
    artifacts: Vec<TaskArtifact>,
}

async fn execute_task(log_sink: &WorkerLogSink, lease: &TaskLease) -> Result<ExecutionOutput> {
    let started_at = SystemTime::now();
    let mut output = if let Some(openjd) = &lease.task.openjd {
        execute_openjd(log_sink, &lease.task, openjd).await?
    } else {
        execute_command(log_sink, lease).await?
    };
    output.artifacts = collect_artifacts(&lease.task, &output.stdout, &output.stderr, started_at);
    Ok(output)
}

async fn execute_command(log_sink: &WorkerLogSink, lease: &TaskLease) -> Result<ExecutionOutput> {
    let command = &lease.task.command;
    let mut child = Command::new(&command.executable);
    child.args(&command.args);
    child.envs(&command.env);
    if let Some(working_dir) = &command.working_dir {
        child.current_dir(working_dir);
    }
    child.stdout(Stdio::piped());
    child.stderr(Stdio::piped());

    let mut child = child.spawn().context("task process failed to spawn")?;
    let stdout = child
        .stdout
        .take()
        .context("task stdout pipe was unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("task stderr pipe was unavailable")?;
    let stdout_task = tokio::spawn(read_stream_lines(
        stdout,
        log_sink.clone(),
        lease.task.job_id,
        lease.task.id,
        "stdout",
        LogLevel::Info,
    ));
    let stderr_task = tokio::spawn(read_stream_lines(
        stderr,
        log_sink.clone(),
        lease.task.job_id,
        lease.task.id,
        "stderr",
        LogLevel::Error,
    ));

    let status = if let Some(timeout_seconds) = command.timeout_seconds {
        match tokio::time::timeout(Duration::from_secs(timeout_seconds), child.wait()).await {
            Ok(status) => status.context("task process failed")?,
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(anyhow::anyhow!("task timed out"));
            }
        }
    } else {
        child.wait().await.context("task process failed")?
    };

    let stdout = stdout_task
        .await
        .context("stdout reader task failed")?
        .context("stdout read failed")?;
    let stderr = stderr_task
        .await
        .context("stderr reader task failed")?
        .context("stderr read failed")?;

    Ok(ExecutionOutput {
        exit_code: status.code().unwrap_or(-1),
        stdout,
        stderr,
        artifacts: Vec::new(),
    })
}

async fn execute_openjd(
    log_sink: &WorkerLogSink,
    task: &Task,
    openjd: &OpenJdRuntimeTask,
) -> Result<ExecutionOutput> {
    let tmp = tempfile::TempDir::new().context("create OpenJD session directory")?;
    let job_parameters = openjd
        .job_parameters
        .iter()
        .map(|(name, value)| {
            value
                .to_job_parameter()
                .map(|value| (name.clone(), value))
                .map_err(anyhow::Error::msg)
        })
        .collect::<Result<_>>()?;
    let mut task_parameters = TaskParameterSet::new();
    for (name, value) in &openjd.task_parameters {
        task_parameters.insert(
            name.clone(),
            value.to_task_parameter().map_err(anyhow::Error::msg)?,
        );
    }

    let extensions = openjd
        .extensions
        .iter()
        .filter_map(|extension| extension.parse::<ModelExtension>().ok())
        .collect();
    let profile =
        Some(ModelProfile::new(SpecificationRevision::V2023_09).with_extensions(extensions));
    let path_mapping_rules = openjd
        .path_mapping_rules
        .iter()
        .cloned()
        .map(Into::into)
        .collect::<Vec<_>>();
    let callback_log_sink = log_sink.clone();
    let callback_job_id = task.job_id;
    let callback_task_id = task.id;
    let callback: OpenJdStatusCallback =
        Box::new(move |session_id: &str, status: &ActionStatus| {
            let Some(handle) = tokio::runtime::Handle::try_current().ok() else {
                return;
            };
            let log_sink = callback_log_sink.clone();
            let message = format_openjd_action_status(session_id, status);
            handle.spawn(async move {
                log_sink
                    .post_line(
                        LogLevel::Info,
                        Some("openjd-status".to_string()),
                        message,
                        Some(callback_job_id),
                        Some(callback_task_id),
                    )
                    .await;
            });
        });

    let config = SessionConfig {
        session_id: format!("renderacre-{}", uuid::Uuid::new_v4()),
        job_parameter_values: job_parameters,
        path_mapping_rules: if path_mapping_rules.is_empty() {
            None
        } else {
            Some(path_mapping_rules)
        },
        retain_working_dir: false,
        callback: Some(callback),
        os_env_vars: None,
        session_root_directory: Some(tmp.path().to_path_buf()),
        user: None,
        profile,
        cancel_token: None,
        sticky_bit_policy: StickyBitPolicy::Disabled,
        debug_collect_stdout: true,
        echo_openjd_directives: true,
    };
    let mut session = Session::with_config(config).context("create OpenJD session")?;

    let step_script: openjd_model::job::StepScript =
        serde_json::from_value(openjd.step_script.clone()).context("decode OpenJD step script")?;
    let step_resolved = openjd
        .step_resolved_symtab
        .clone()
        .map(SerializedSymbolTable::from_value);

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut job_env_entries = Vec::new();
    for env in &openjd.job_environments {
        let environment: openjd_model::job::Environment =
            serde_json::from_value(env.environment.clone())
                .context("decode OpenJD job environment")?;
        let resolved = env
            .resolved_symtab
            .clone()
            .map(SerializedSymbolTable::from_value);
        match session
            .enter_environment_with_output(&environment, resolved.as_ref(), None, None)
            .await
        {
            Ok((id, output)) => {
                log_sink
                    .post_lines(task.job_id, task.id, "stdout", LogLevel::Info, &output)
                    .await;
                stdout.push_str(&output);
                job_env_entries.push((id, resolved));
            }
            Err(err) => {
                let message = format!("OpenJD job environment failed: {err}\n");
                log_sink
                    .post_lines(task.job_id, task.id, "stderr", LogLevel::Error, &message)
                    .await;
                stderr.push_str(&message);
                session.cleanup();
                return Ok(ExecutionOutput {
                    exit_code: -1,
                    stdout,
                    stderr,
                    artifacts: Vec::new(),
                });
            }
        }
    }

    let mut step_env_ids = Vec::new();
    for env in &openjd.step_environments {
        let environment: openjd_model::job::Environment =
            serde_json::from_value(env.clone()).context("decode OpenJD step environment")?;
        match session
            .enter_environment_with_output(&environment, step_resolved.as_ref(), None, None)
            .await
        {
            Ok((id, output)) => {
                log_sink
                    .post_lines(task.job_id, task.id, "stdout", LogLevel::Info, &output)
                    .await;
                stdout.push_str(&output);
                step_env_ids.push(id);
            }
            Err(err) => {
                let message = format!("OpenJD step environment failed: {err}\n");
                log_sink
                    .post_lines(task.job_id, task.id, "stderr", LogLevel::Error, &message)
                    .await;
                stderr.push_str(&message);
                session.cleanup();
                return Ok(ExecutionOutput {
                    exit_code: -1,
                    stdout,
                    stderr,
                    artifacts: Vec::new(),
                });
            }
        }
    }

    let result = session
        .run_task(
            &step_script,
            if task_parameters.is_empty() {
                None
            } else {
                Some(&task_parameters)
            },
            step_resolved.as_ref(),
            None,
        )
        .await;
    let exit_code = match result {
        Ok(result) => {
            log_sink
                .post_lines(
                    task.job_id,
                    task.id,
                    "stdout",
                    LogLevel::Info,
                    &result.stdout,
                )
                .await;
            stdout.push_str(&result.stdout);
            match result.state {
                ActionState::Success => 0,
                _ => result.exit_code.unwrap_or(-1),
            }
        }
        Err(err) => {
            let message = format!("OpenJD task failed: {err}\n");
            log_sink
                .post_lines(task.job_id, task.id, "stderr", LogLevel::Error, &message)
                .await;
            stderr.push_str(&message);
            -1
        }
    };

    for id in step_env_ids.iter().rev() {
        if let Ok(output) = session
            .exit_environment(id, step_resolved.as_ref(), true, None)
            .await
        {
            log_sink
                .post_lines(task.job_id, task.id, "stdout", LogLevel::Info, &output)
                .await;
            stdout.push_str(&output);
        }
    }
    for (id, resolved) in job_env_entries.iter().rev() {
        if let Ok(output) = session
            .exit_environment(id, resolved.as_ref(), true, None)
            .await
        {
            log_sink
                .post_lines(task.job_id, task.id, "stdout", LogLevel::Info, &output)
                .await;
            stdout.push_str(&output);
        }
    }
    session.cleanup();

    Ok(ExecutionOutput {
        exit_code,
        stdout,
        stderr,
        artifacts: Vec::new(),
    })
}

fn format_openjd_action_status(session_id: &str, status: &ActionStatus) -> String {
    let mut parts = vec![format!(
        "OpenJD session {session_id} action state={}",
        status.state
    )];
    if let Some(progress) = status.progress {
        parts.push(format!("progress={progress:.2}"));
    }
    if let Some(message) = &status.status_message {
        parts.push(format!("status={message}"));
    }
    if let Some(message) = &status.fail_message {
        parts.push(format!("fail={message}"));
    }
    if let Some(exit_code) = status.exit_code {
        parts.push(format!("exit_code={exit_code}"));
    }
    parts.join(" ")
}

async fn read_stream_lines<R>(
    stream: R,
    log_sink: WorkerLogSink,
    job_id: uuid::Uuid,
    task_id: uuid::Uuid,
    stream_name: &'static str,
    level: LogLevel,
) -> Result<String>
where
    R: AsyncRead + Unpin,
{
    let mut captured = String::new();
    let mut lines = BufReader::new(stream).lines();
    while let Some(line) = lines.next_line().await? {
        captured.push_str(&line);
        captured.push('\n');
        log_sink
            .post_line(
                level.clone(),
                Some(stream_name.to_string()),
                line,
                Some(job_id),
                Some(task_id),
            )
            .await;
    }
    Ok(captured)
}

fn collect_artifacts(
    task: &Task,
    stdout: &str,
    stderr: &str,
    started_at: SystemTime,
) -> Vec<TaskArtifact> {
    let mut exact_paths = artifact_directive_paths(stdout)
        .into_iter()
        .chain(artifact_directive_paths(stderr))
        .collect::<Vec<_>>();
    let mut directory_paths = task.artifact_paths.clone();
    exact_paths.extend(
        task.artifact_paths
            .iter()
            .filter(|path| path.is_file())
            .cloned(),
    );
    directory_paths.retain(|path| path.is_dir());

    let mut artifacts = Vec::new();
    let mut seen = HashSet::new();
    for path in exact_paths {
        push_artifact(&path, &mut artifacts, &mut seen);
    }
    let since = started_at
        .checked_sub(Duration::from_secs(2))
        .unwrap_or(started_at);
    for path in directory_paths {
        collect_modified_files(&path, since, &mut artifacts, &mut seen);
    }
    artifacts
}

fn artifact_directive_paths(buffer: &str) -> Vec<PathBuf> {
    buffer
        .lines()
        .filter_map(|line| line.trim().strip_prefix("RENDERACRE_ARTIFACT="))
        .map(|path| PathBuf::from(path.trim()))
        .collect()
}

fn collect_modified_files(
    root: &Path,
    since: SystemTime,
    artifacts: &mut Vec<TaskArtifact>,
    seen: &mut HashSet<PathBuf>,
) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        if artifacts.len() >= 128 {
            return;
        }
        let Ok(read_dir) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata
                .modified()
                .map(|modified| modified >= since)
                .unwrap_or(false)
            {
                push_artifact(&path, artifacts, seen);
            }
        }
    }
}

fn push_artifact(path: &Path, artifacts: &mut Vec<TaskArtifact>, seen: &mut HashSet<PathBuf>) {
    let Ok(path) = path.canonicalize() else {
        return;
    };
    if !seen.insert(path.clone()) {
        return;
    }
    let Ok(metadata) = std::fs::metadata(&path) else {
        return;
    };
    if !metadata.is_file() {
        return;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("artifact")
        .to_string();
    artifacts.push(TaskArtifact {
        kind: artifact_kind(&name),
        name,
        path,
        size_bytes: metadata.len(),
        modified_at: metadata
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from),
    });
}

fn artifact_kind(name: &str) -> ArtifactKind {
    match name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" | "jpg" | "jpeg" | "webp" | "gif" => ArtifactKind::Image,
        "ma" | "mb" | "blend" | "usd" | "usda" | "usdc" => ArtifactKind::Scene,
        "txt" | "log" => ArtifactKind::Log,
        _ => ArtifactKind::File,
    }
}

fn tail_text(bytes: &[u8], max_len: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let start = text.len().saturating_sub(max_len);
    text[start..].to_string()
}

fn parse_label(value: &str) -> std::result::Result<(String, String), String> {
    let Some((key, value)) = value.split_once('=') else {
        return Err("labels must use key=value syntax".to_string());
    };
    Ok((key.to_string(), value.to_string()))
}

fn default_worker_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "local-worker".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_overrides_worker_defaults() {
        let config_path = write_config(
            "worker",
            r#"
controller: http://renderacre-controller:7878
name: render-node-01
labels:
  app: blender
  pool: lighting
slots: 4
poll_seconds: 5
lease_renew_seconds: 20
"#,
        );

        let args = Args::load_from([
            "renderacre-worker",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .expect("config should load");

        assert_eq!(args.controller, "http://renderacre-controller:7878");
        assert_eq!(args.name.as_deref(), Some("render-node-01"));
        assert_eq!(
            args.labels,
            vec![
                ("app".to_string(), "blender".to_string()),
                ("pool".to_string(), "lighting".to_string())
            ]
        );
        assert_eq!(args.slots, 4);
        assert_eq!(args.poll_seconds, 5);
        assert_eq!(args.lease_renew_seconds, 20);
        let _ = std::fs::remove_file(config_path);
    }

    #[test]
    fn command_line_values_override_worker_config() {
        let config_path = write_config(
            "worker-cli",
            r#"
controller: http://renderacre-controller:7878
name: render-node-01
labels:
  app: blender
slots: 4
"#,
        );

        let args = Args::load_from([
            "renderacre-worker",
            "--config",
            config_path.to_str().unwrap(),
            "--controller",
            "http://127.0.0.1:7878",
            "--label",
            "app=maya",
            "--slots",
            "2",
        ])
        .expect("config should load");

        assert_eq!(args.controller, "http://127.0.0.1:7878");
        assert_eq!(args.labels, vec![("app".to_string(), "maya".to_string())]);
        assert_eq!(args.slots, 2);
        assert_eq!(args.name.as_deref(), Some("render-node-01"));
        let _ = std::fs::remove_file(config_path);
    }

    fn write_config(name: &str, content: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("renderacre-{name}-{}.yaml", uuid::Uuid::new_v4()));
        std::fs::write(&path, content).expect("config should write");
        path
    }
}
