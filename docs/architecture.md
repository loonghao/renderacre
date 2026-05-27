# Renderacre architecture

Renderacre is shaped like a small Deadline-style farm: one controller owns the queue and worker registry, workers lease tasks and report results, and Python/DCC submitters talk to the same HTTP contract.

- `farm-controller`: central HTTP API, worker registry, job queue, retry accounting, and lease protocol.
- `farm-worker`: long-running agent that polls the controller, executes commands, and reports completion.
- `farm-core`: shared domain models, scheduling rules, retry/lease handling, and OpenJD adaptation.
- `renderacre`: PyO3/maturin extension module for Python submitters and pipeline tools.

## Control plane

The first implementation uses an in-memory scheduler so the API and worker protocol can stabilize quickly. The storage boundary is deliberately inside `InMemoryScheduler`; replacing it with SQLite/Postgres later should not change the remote API or Python API.

Core endpoints:

- `POST /v1/jobs`: submit a direct task job or an OpenJD-backed job.
- `GET /v1/jobs/{job_id}`: inspect state and task output tails.
- `POST /v1/workers/register`: register a worker.
- `POST /v1/workers/{worker_id}/heartbeat`: keep worker online.
- `POST /v1/workers/{worker_id}/lease`: get the next runnable task.
- `POST /v1/tasks/{task_id}/started`: mark a leased task as running.
- `POST /v1/tasks/{task_id}/complete`: report result and trigger retry/final state.

## OpenJD path

The controller accepts an OpenJD template bundle under `openjd.template_yaml`. It uses the official OpenJD Rust crates instead of a hand-written parser:

- `openjd-model` parses YAML/JSON, validates the template, preprocesses typed job parameters, creates the resolved job model, and expands step parameter spaces.
- Renderacre converts each resolved step/task combination into a farm task while preserving OpenJD runtime context such as parameter values, path mappings, environments, embedded files, and supported extensions.
- `openjd-sessions` runs the task on the worker, including environment enter/exit actions and OpenJD stdout directive handling.

The first storage backend is intentionally in-memory, but the OpenJD conversion boundary is separate from storage so a durable queue can be added without changing Python submitters or worker execution.

## Python path

`crates/farm-python` builds the `renderacre` extension through maturin/PyO3. The wheel is configured as `cp37-abi3`, so one wheel per platform supports CPython 3.7 and newer.

The Python API is deliberately small:

- `command_job(...)` builds direct command payloads.
- `openjd_job(...)` builds OpenJD payloads from template text and JSON parameters.
- `submit_job(...)` posts a payload to a controller and returns the response JSON.

Keeping Python at the API edge lets DCC submitters stay pleasant while Rust owns scheduling, OpenJD validation, worker leases, and task execution.

## Deployment shape

The recommended first production shape is a private controller behind an authenticated proxy plus one worker process per render node. OpenJD `PATH` parameters should point at shared storage for scenes, scripts, and output directories. The current scheduler keeps state in memory; SQLite/Postgres/NATS can replace `InMemoryScheduler` later behind the same REST and Python contracts.
