use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use farm_core::{
    OpenJdRuntimeTask, TaskComplete, TaskLease, TaskStarted, WorkerCapacity, WorkerId, WorkerInfo,
    WorkerRegister,
};
use openjd_expr::SerializedSymbolTable;
use openjd_model::{ModelExtension, ModelProfile, SpecificationRevision, TaskParameterSet};
use openjd_sessions::{ActionState, Session, SessionConfig, StickyBitPolicy};
use tokio::process::Command;

#[derive(Debug, Parser)]
struct Args {
    #[arg(
        long,
        env = "RFARM_CONTROLLER",
        default_value = "http://127.0.0.1:7878"
    )]
    controller: String,
    #[arg(long)]
    name: Option<String>,
    #[arg(long = "label", value_parser = parse_label)]
    labels: Vec<(String, String)>,
    #[arg(long, default_value_t = 1)]
    slots: u32,
    #[arg(long, default_value_t = 2)]
    poll_seconds: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "farm_worker=info".to_string()),
        )
        .init();

    let args = Args::parse();
    let client = reqwest::Client::new();
    let controller = args.controller.trim_end_matches('/').to_string();
    let worker = register_worker(&client, &controller, &args).await?;
    tracing::info!(worker_id = %worker.id, "worker registered");

    loop {
        heartbeat(&client, &controller, worker.id).await?;
        match lease_task(&client, &controller, worker.id).await? {
            Some(lease) => run_lease(&client, &controller, worker.id, lease).await?,
            None => tokio::time::sleep(Duration::from_secs(args.poll_seconds)).await,
        }
    }
}

async fn register_worker(
    client: &reqwest::Client,
    controller: &str,
    args: &Args,
) -> Result<WorkerInfo> {
    let labels = args.labels.iter().cloned().collect::<HashMap<_, _>>();
    let name = args.name.clone().unwrap_or_else(default_worker_name);
    let worker = client
        .post(format!("{controller}/v1/workers/register"))
        .json(&WorkerRegister {
            name,
            labels,
            capacity: WorkerCapacity { slots: args.slots },
        })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(worker)
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
    worker_id: WorkerId,
    lease: TaskLease,
) -> Result<()> {
    tracing::info!(task_id = %lease.task.id, task = %lease.task.name, "running task");
    client
        .post(format!("{controller}/v1/tasks/{}/started", lease.task.id))
        .json(&TaskStarted {
            worker_id,
            lease_token: lease.lease_token.clone(),
        })
        .send()
        .await?
        .error_for_status()?;

    let output = execute_task(&lease).await;
    let completion = match output {
        Ok(output) => TaskComplete {
            worker_id,
            lease_token: lease.lease_token,
            exit_code: output.exit_code,
            stdout_tail: Some(tail_text(output.stdout.as_bytes(), 8192)),
            stderr_tail: Some(tail_text(output.stderr.as_bytes(), 8192)),
        },
        Err(err) => TaskComplete {
            worker_id,
            lease_token: lease.lease_token,
            exit_code: -1,
            stdout_tail: None,
            stderr_tail: Some(err.to_string()),
        },
    };

    client
        .post(format!("{controller}/v1/tasks/{}/complete", lease.task.id))
        .json(&completion)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

struct ExecutionOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

async fn execute_task(lease: &TaskLease) -> Result<ExecutionOutput> {
    if let Some(openjd) = &lease.task.openjd {
        execute_openjd(openjd).await
    } else {
        execute_command(lease).await
    }
}

async fn execute_command(lease: &TaskLease) -> Result<ExecutionOutput> {
    let command = &lease.task.command;
    let mut child = Command::new(&command.executable);
    child.args(&command.args);
    child.envs(&command.env);
    if let Some(working_dir) = &command.working_dir {
        child.current_dir(working_dir);
    }

    let future = child.output();
    if let Some(timeout_seconds) = command.timeout_seconds {
        let output = tokio::time::timeout(Duration::from_secs(timeout_seconds), future)
            .await
            .context("task timed out")?
            .context("task process failed")?;
        Ok(process_output(output))
    } else {
        let output = future.await.context("task process failed")?;
        Ok(process_output(output))
    }
}

async fn execute_openjd(openjd: &OpenJdRuntimeTask) -> Result<ExecutionOutput> {
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

    let config = SessionConfig {
        session_id: format!("renderacre-{}", uuid::Uuid::new_v4()),
        job_parameter_values: job_parameters,
        path_mapping_rules: if path_mapping_rules.is_empty() {
            None
        } else {
            Some(path_mapping_rules)
        },
        retain_working_dir: false,
        callback: None,
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
                stdout.push_str(&output);
                job_env_entries.push((id, resolved));
            }
            Err(err) => {
                stderr.push_str(&format!("OpenJD job environment failed: {err}\n"));
                session.cleanup();
                return Ok(ExecutionOutput {
                    exit_code: -1,
                    stdout,
                    stderr,
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
                stdout.push_str(&output);
                step_env_ids.push(id);
            }
            Err(err) => {
                stderr.push_str(&format!("OpenJD step environment failed: {err}\n"));
                session.cleanup();
                return Ok(ExecutionOutput {
                    exit_code: -1,
                    stdout,
                    stderr,
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
            stdout.push_str(&result.stdout);
            match result.state {
                ActionState::Success => 0,
                _ => result.exit_code.unwrap_or(-1),
            }
        }
        Err(err) => {
            stderr.push_str(&format!("OpenJD task failed: {err}\n"));
            -1
        }
    };

    for id in step_env_ids.iter().rev() {
        if let Ok(output) = session
            .exit_environment(id, step_resolved.as_ref(), true, None)
            .await
        {
            stdout.push_str(&output);
        }
    }
    for (id, resolved) in job_env_entries.iter().rev() {
        if let Ok(output) = session
            .exit_environment(id, resolved.as_ref(), true, None)
            .await
        {
            stdout.push_str(&output);
        }
    }
    session.cleanup();

    Ok(ExecutionOutput {
        exit_code,
        stdout,
        stderr,
    })
}

fn process_output(output: std::process::Output) -> ExecutionOutput {
    ExecutionOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
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
