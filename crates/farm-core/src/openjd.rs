use std::collections::HashMap;
use std::path::PathBuf;

use openjd_model::create_job::{create_job, preprocess_job_parameters};
use openjd_model::step_param_space::StepParameterSpaceIterator;
use openjd_model::template::parse::{decode_job_template, document_string_to_object, DocumentType};
use openjd_model::{CallerLimits, JobParameterInputValues, ModelExtension, PathParameterOptions};

use crate::models::{
    json_to_openjd_input_value, CommandSpec, OpenJdAmountRequirement, OpenJdAttributeRequirement,
    OpenJdEnvironmentRuntime, OpenJdParameterValue, OpenJdRuntimeTask, OpenJdSubmit,
    TaskRequirements, TaskSubmit,
};
use crate::scheduler::FarmError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenJdSummary {
    pub specification_version: String,
    pub name: Option<String>,
    pub step_count: usize,
    pub task_count: usize,
    pub extensions: Vec<String>,
}

pub fn summarize_openjd(bundle: &OpenJdSubmit) -> Result<OpenJdSummary, FarmError> {
    let decoded = decode_bundle(bundle)?;
    Ok(OpenJdSummary {
        specification_version: decoded.template.specification_version.clone(),
        name: Some(decoded.template.name().raw().to_string()),
        step_count: decoded.job.steps.len(),
        task_count: expand_openjd_tasks(bundle, &decoded)?.len(),
        extensions: decoded.extensions,
    })
}

pub fn openjd_to_tasks(bundle: &OpenJdSubmit) -> Result<Vec<TaskSubmit>, FarmError> {
    let decoded = decode_bundle(bundle)?;
    expand_openjd_tasks(bundle, &decoded)
}

struct DecodedOpenJd {
    template: openjd_model::template::JobTemplate,
    job: openjd_model::job::Job,
    job_parameters: openjd_model::JobParameterValues,
    extensions: Vec<String>,
}

fn decode_bundle(bundle: &OpenJdSubmit) -> Result<DecodedOpenJd, FarmError> {
    let document = document_string_to_object(
        &bundle.template_yaml,
        DocumentType::Yaml,
        &CallerLimits::default(),
    )
    .map_err(|err| FarmError::InvalidSubmission(format!("OpenJD template parse failed: {err}")))?;

    let supported_extensions = supported_extensions(bundle);
    let supported_refs = supported_extensions
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let template = decode_job_template(document, Some(&supported_refs), &CallerLimits::default())
        .map_err(|err| {
        FarmError::InvalidSubmission(format!("OpenJD template validation failed: {err}"))
    })?;

    let job_parameter_inputs = job_parameter_inputs(bundle)?;
    let path_options = path_parameter_options(bundle)?;
    let job_parameters = preprocess_job_parameters(
        &template,
        &job_parameter_inputs,
        &[],
        &path_options.as_openjd_options(),
    )
    .map_err(|err| {
        FarmError::InvalidSubmission(format!("OpenJD parameter validation failed: {err}"))
    })?;

    let job = create_job(
        &template,
        &job_parameters,
        &template.default_validation_context(),
    )
    .map_err(|err| FarmError::InvalidSubmission(format!("OpenJD job creation failed: {err}")))?;

    Ok(DecodedOpenJd {
        template,
        extensions: supported_extensions,
        job,
        job_parameters,
    })
}

fn expand_openjd_tasks(
    bundle: &OpenJdSubmit,
    decoded: &DecodedOpenJd,
) -> Result<Vec<TaskSubmit>, FarmError> {
    if decoded.job.steps.is_empty() {
        return Err(FarmError::InvalidSubmission(
            "OpenJD template does not define any steps".to_string(),
        ));
    }

    let job_parameters = decoded
        .job_parameters
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                OpenJdParameterValue::from_job_parameter(value),
            )
        })
        .collect::<HashMap<_, _>>();
    let job_environments = decoded
        .job
        .job_environments
        .as_ref()
        .map(|envs| {
            envs.iter()
                .map(|env| {
                    Ok(OpenJdEnvironmentRuntime {
                        environment: serde_json::to_value(env).map_err(json_error)?,
                        resolved_symtab: env
                            .resolved_symtab
                            .as_ref()
                            .map(serde_json::to_value)
                            .transpose()
                            .map_err(json_error)?,
                    })
                })
                .collect::<Result<Vec<_>, FarmError>>()
        })
        .transpose()?
        .unwrap_or_default();

    let mut task_names_by_step = HashMap::<String, Vec<String>>::new();
    let mut tasks = Vec::new();

    for step in &decoded.job.steps {
        let task_parameter_sets = task_parameter_sets(step)?;
        for (index, task_parameters) in task_parameter_sets.into_iter().enumerate() {
            let task_parameters = task_parameters
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        OpenJdParameterValue::from_task_parameter(value),
                    )
                })
                .collect::<HashMap<_, _>>();
            let task_name = openjd_task_name(&step.name, index, &task_parameters);
            let dependencies = step
                .dependencies
                .as_ref()
                .map(|deps| {
                    deps.iter()
                        .flat_map(|dep| {
                            task_names_by_step
                                .get(&dep.depends_on)
                                .cloned()
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let action = &step.script.actions.on_run;
            let command = CommandSpec {
                executable: action.command.raw().to_string(),
                args: action
                    .args
                    .as_ref()
                    .map(|args| args.iter().map(|arg| arg.raw().to_string()).collect())
                    .unwrap_or_default(),
                env: HashMap::new(),
                working_dir: bundle.asset_root.clone(),
                timeout_seconds: None,
            };
            let openjd = OpenJdRuntimeTask {
                job_name: decoded.job.name.clone(),
                step_name: step.name.clone(),
                extensions: decoded.extensions.clone(),
                job_parameters: job_parameters.clone(),
                task_parameters,
                path_mapping_rules: bundle.path_mapping_rules.clone(),
                job_environments: job_environments.clone(),
                step_environments: step
                    .step_environments
                    .as_ref()
                    .map(|envs| {
                        envs.iter()
                            .map(serde_json::to_value)
                            .collect::<Result<Vec<_>, _>>()
                            .map_err(json_error)
                    })
                    .transpose()?
                    .unwrap_or_default(),
                step_script: serde_json::to_value(&step.script).map_err(json_error)?,
                step_resolved_symtab: step
                    .resolved_symtab
                    .as_ref()
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(json_error)?,
            };

            task_names_by_step
                .entry(step.name.clone())
                .or_default()
                .push(task_name.clone());
            tasks.push(TaskSubmit {
                name: task_name,
                command,
                dependencies,
                requirements: step
                    .host_requirements
                    .as_ref()
                    .map(openjd_host_requirements)
                    .unwrap_or_default(),
                limits: Vec::new(),
                max_retries: None,
                artifact_paths: infer_artifact_paths(&job_parameters),
                openjd: Some(openjd),
            });
        }
    }

    Ok(tasks)
}

fn infer_artifact_paths(job_parameters: &HashMap<String, OpenJdParameterValue>) -> Vec<PathBuf> {
    job_parameters
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.to_ascii_lowercase();
            if !(lower.contains("output") || lower.contains("artifact")) {
                return None;
            }
            value.value.as_str().map(PathBuf::from)
        })
        .collect()
}

fn openjd_host_requirements(value: &openjd_model::job::HostRequirements) -> TaskRequirements {
    TaskRequirements {
        amounts: value
            .amounts
            .as_ref()
            .map(|amounts| {
                amounts
                    .iter()
                    .map(|amount| OpenJdAmountRequirement {
                        name: amount.name.clone(),
                        min: amount.min,
                        max: amount.max,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        attributes: value
            .attributes
            .as_ref()
            .map(|attributes| {
                attributes
                    .iter()
                    .map(|attribute| OpenJdAttributeRequirement {
                        name: attribute.name.clone(),
                        any_of: attribute.any_of.clone().unwrap_or_default(),
                        all_of: attribute.all_of.clone().unwrap_or_default(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ..Default::default()
    }
}

fn task_parameter_sets(
    step: &openjd_model::job::Step,
) -> Result<Vec<openjd_model::TaskParameterSet>, FarmError> {
    if let Some(parameter_space) = &step.parameter_space {
        StepParameterSpaceIterator::new(parameter_space)
            .map_err(|err| {
                FarmError::InvalidSubmission(format!(
                    "OpenJD step '{}' parameter space is invalid: {err}",
                    step.name
                ))
            })
            .map(|iter| iter.collect())
    } else {
        Ok(vec![openjd_model::TaskParameterSet::new()])
    }
}

fn openjd_task_name(
    step_name: &str,
    index: usize,
    task_parameters: &HashMap<String, OpenJdParameterValue>,
) -> String {
    if task_parameters.is_empty() {
        return step_name.to_string();
    }

    let mut keys = task_parameters.keys().collect::<Vec<_>>();
    keys.sort();
    let suffix = keys
        .into_iter()
        .map(|key| {
            let value = task_parameters
                .get(key)
                .map(|value| value.value.to_string())
                .unwrap_or_default();
            format!("{key}={}", value.trim_matches('"'))
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{step_name}[{suffix}]#{index}")
}

fn job_parameter_inputs(bundle: &OpenJdSubmit) -> Result<JobParameterInputValues, FarmError> {
    bundle
        .parameters
        .iter()
        .map(|(name, value)| {
            json_to_openjd_input_value(value)
                .map(|value| (name.clone(), value))
                .map_err(|err| {
                    FarmError::InvalidSubmission(format!(
                        "OpenJD parameter '{name}' is invalid: {err}"
                    ))
                })
        })
        .collect()
}

struct PathOptions {
    template_dir: String,
    current_working_dir: String,
    path_format: openjd_expr::PathFormat,
}

impl PathOptions {
    fn as_openjd_options(&self) -> PathParameterOptions<'_> {
        PathParameterOptions {
            job_template_dir: &self.template_dir,
            current_working_dir: &self.current_working_dir,
            allow_template_dir_walk_up: false,
            path_format: self.path_format,
            allow_uri_path_values: true,
        }
    }
}

fn path_parameter_options(bundle: &OpenJdSubmit) -> Result<PathOptions, FarmError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let template_dir = bundle
        .template_dir
        .as_ref()
        .or(bundle.asset_root.as_ref())
        .unwrap_or(&cwd)
        .to_string_lossy()
        .to_string();
    let current_working_dir = bundle
        .current_working_dir
        .as_ref()
        .or(bundle.asset_root.as_ref())
        .unwrap_or(&cwd)
        .to_string_lossy()
        .to_string();

    Ok(PathOptions {
        template_dir,
        current_working_dir,
        path_format: openjd_expr::PathFormat::host(),
    })
}

fn supported_extensions(bundle: &OpenJdSubmit) -> Vec<String> {
    if bundle.supported_extensions.is_empty() {
        ModelExtension::ALL
            .iter()
            .map(|extension| extension.as_str().to_string())
            .collect()
    } else {
        bundle.supported_extensions.clone()
    }
}

fn json_error(error: serde_json::Error) -> FarmError {
    FarmError::InvalidSubmission(format!(
        "OpenJD runtime payload serialization failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use serde_json::json;

    use super::*;

    #[test]
    fn converts_simple_openjd_template_to_runtime_task() {
        let mut parameters = HashMap::new();
        parameters.insert("Message".to_string(), json!("hello"));
        let bundle = OpenJdSubmit {
            template_yaml: r#"
specificationVersion: jobtemplate-2023-09
name: Hello
parameterDefinitions:
  - name: Message
    type: STRING
steps:
  - name: Echo
    script:
      actions:
        onRun:
          command: powershell
          args:
            - -NoProfile
            - -Command
            - Write-Output '{{ Param.Message }}'
"#
            .to_string(),
            parameters,
            asset_root: None,
            supported_extensions: Vec::new(),
            path_mapping_rules: Vec::new(),
            template_dir: None,
            current_working_dir: None,
        };

        let tasks = openjd_to_tasks(&bundle).expect("OpenJD should parse");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "Echo");
        assert!(tasks[0].openjd.is_some());
    }

    #[test]
    fn expands_openjd_parameter_space() {
        let bundle = OpenJdSubmit {
            template_yaml: r#"
specificationVersion: jobtemplate-2023-09
name: Frames
steps:
  - name: Render
    parameterSpace:
      taskParameterDefinitions:
        - name: Frame
          type: INT
          range: "1-3"
    script:
      actions:
        onRun:
          command: python
          args:
            - -c
            - print('{{ Task.Param.Frame }}')
"#
            .to_string(),
            parameters: HashMap::new(),
            asset_root: None,
            supported_extensions: Vec::new(),
            path_mapping_rules: Vec::new(),
            template_dir: None,
            current_working_dir: None,
        };

        let tasks = openjd_to_tasks(&bundle).expect("OpenJD should parse");
        assert_eq!(tasks.len(), 3);
        assert!(tasks.iter().any(|task| task.name.contains("Frame=1")));
        assert!(tasks.iter().any(|task| task.name.contains("Frame=2")));
        assert!(tasks.iter().any(|task| task.name.contains("Frame=3")));
    }

    #[test]
    fn preserves_openjd_host_requirements_for_scheduler() {
        let bundle = OpenJdSubmit {
            template_yaml: r#"
specificationVersion: jobtemplate-2023-09
name: HostRequirements
steps:
  - name: LinuxRender
    hostRequirements:
      amounts:
        - name: amount.worker.vcpu
          min: 2
      attributes:
        - name: attr.worker.os.family
          anyOf: [linux]
    script:
      actions:
        onRun:
          command: echo
"#
            .to_string(),
            parameters: HashMap::new(),
            asset_root: None,
            supported_extensions: Vec::new(),
            path_mapping_rules: Vec::new(),
            template_dir: None,
            current_working_dir: None,
        };

        let tasks = openjd_to_tasks(&bundle).expect("OpenJD should parse");
        assert_eq!(tasks.len(), 1);
        let requirements = &tasks[0].requirements;
        assert_eq!(requirements.amounts[0].name, "amount.worker.vcpu");
        assert_eq!(requirements.amounts[0].min, Some(2.0));
        assert_eq!(requirements.attributes[0].name, "attr.worker.os.family");
        assert_eq!(requirements.attributes[0].any_of, vec!["linux"]);
    }

    #[test]
    fn summarizes_repository_python_example() {
        let mut bundle = example_bundle("examples/openjd_python_frames.yaml");
        bundle
            .parameters
            .insert("Message".to_string(), json!("from-test"));

        let summary = summarize_openjd(&bundle).expect("example should summarize");
        assert_eq!(summary.name.as_deref(), Some("PythonFrameSmoke"));
        assert_eq!(summary.step_count, 1);
        assert_eq!(summary.task_count, 5);
    }

    #[test]
    fn converts_repository_dcc_examples() {
        let root = repo_root();

        let mut blender = example_bundle("examples/dcc/blender_render_openjd.yaml");
        blender
            .parameters
            .insert("BlenderExecutable".to_string(), json!("blender"));
        blender.parameters.insert(
            "ScriptPath".to_string(),
            json!(path_string(
                root.join("examples/dcc/blender_render_task.py")
            )),
        );
        blender.parameters.insert(
            "OutputDir".to_string(),
            json!(path_string(root.join("target/openjd-test/blender"))),
        );

        let blender_tasks = openjd_to_tasks(&blender).expect("Blender example should parse");
        assert_eq!(blender_tasks.len(), 3);
        assert!(blender_tasks.iter().all(|task| task
            .openjd
            .as_ref()
            .is_some_and(|openjd| openjd.step_name == "RenderFrame")));

        let mut maya = example_bundle("examples/dcc/maya_render_openjd.yaml");
        maya.parameters
            .insert("MayaPython".to_string(), json!("mayapy"));
        maya.parameters.insert(
            "ScriptPath".to_string(),
            json!(path_string(root.join("examples/dcc/maya_render_task.py"))),
        );
        maya.parameters.insert(
            "OutputDir".to_string(),
            json!(path_string(root.join("target/openjd-test/maya"))),
        );

        let maya_summary = summarize_openjd(&maya).expect("Maya example should summarize");
        assert_eq!(maya_summary.name.as_deref(), Some("MayaRender"));
        assert_eq!(maya_summary.task_count, 3);
    }

    #[test]
    fn rejects_invalid_openjd_template_with_official_validation() {
        let bundle = OpenJdSubmit {
            template_yaml: "name: MissingVersion".to_string(),
            parameters: HashMap::new(),
            asset_root: None,
            supported_extensions: Vec::new(),
            path_mapping_rules: Vec::new(),
            template_dir: None,
            current_working_dir: None,
        };

        let err = openjd_to_tasks(&bundle).expect_err("template should be rejected");
        assert!(err.to_string().contains("specificationVersion"));
    }

    fn example_bundle(relative_path: &str) -> OpenJdSubmit {
        let path = repo_root().join(relative_path);
        OpenJdSubmit {
            template_yaml: std::fs::read_to_string(&path).expect("example should be readable"),
            parameters: HashMap::new(),
            asset_root: None,
            supported_extensions: Vec::new(),
            path_mapping_rules: Vec::new(),
            template_dir: path.parent().map(Path::to_path_buf),
            current_working_dir: Some(repo_root()),
        }
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root should resolve")
    }

    fn path_string(path: PathBuf) -> String {
        path.to_string_lossy().to_string()
    }
}
