use std::collections::HashMap;

use farm_core::{CommandSpec, JobSubmit, OpenJdSubmit, TaskSubmit};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

#[pyfunction]
#[pyo3(signature = (name, executable, args=None))]
fn command_job(name: String, executable: String, args: Option<Vec<String>>) -> PyResult<String> {
    let job = JobSubmit {
        name,
        priority: 0,
        max_retries: 0,
        openjd: None,
        tasks: vec![TaskSubmit {
            name: "main".to_string(),
            command: CommandSpec {
                executable,
                args: args.unwrap_or_default(),
                env: HashMap::new(),
                working_dir: None,
                timeout_seconds: None,
            },
            dependencies: Vec::new(),
            requirements: Default::default(),
            max_retries: None,
            artifact_paths: Vec::new(),
            openjd: None,
        }],
    };
    serde_json::to_string_pretty(&job).map_err(py_value_error)
}

#[pyfunction]
#[pyo3(signature = (name, template_yaml, parameters_json=None))]
fn openjd_job(
    name: String,
    template_yaml: String,
    parameters_json: Option<String>,
) -> PyResult<String> {
    let parameters = match parameters_json {
        Some(json) => serde_json::from_str(&json).map_err(py_value_error)?,
        None => HashMap::new(),
    };
    let job = JobSubmit {
        name,
        priority: 0,
        max_retries: 0,
        tasks: Vec::new(),
        openjd: Some(OpenJdSubmit {
            template_yaml,
            parameters,
            asset_root: None,
            supported_extensions: Vec::new(),
            path_mapping_rules: Vec::new(),
            template_dir: None,
            current_working_dir: None,
        }),
    };
    serde_json::to_string_pretty(&job).map_err(py_value_error)
}

#[pyfunction]
fn submit_job(controller_url: String, job_json: String) -> PyResult<String> {
    let job: JobSubmit = serde_json::from_str(&job_json).map_err(py_value_error)?;
    let controller_url = controller_url.trim_end_matches('/');
    let response = reqwest::blocking::Client::new()
        .post(format!("{controller_url}/v1/jobs"))
        .json(&job)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(py_runtime_error)?
        .text()
        .map_err(py_runtime_error)?;
    Ok(response)
}

#[pymodule]
fn renderacre(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(command_job, m)?)?;
    m.add_function(wrap_pyfunction!(openjd_job, m)?)?;
    m.add_function(wrap_pyfunction!(submit_job, m)?)?;
    Ok(())
}

fn py_value_error<E: std::fmt::Display>(err: E) -> PyErr {
    PyValueError::new_err(err.to_string())
}

fn py_runtime_error<E: std::fmt::Display>(err: E) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}
