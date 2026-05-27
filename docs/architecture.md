# Renderacre architecture

Renderacre is shaped like a small Deadline-style farm: one controller owns the queue and worker registry, workers lease tasks and report results, and Python/DCC submitters talk to the same HTTP contract.

- `farm-controller`: central HTTP API, worker registry, job queue, retry accounting, and lease protocol.
- `farm-worker`: long-running agent that polls the controller, executes commands, and reports completion.
- `farm-core`: shared domain models, scheduling rules, retry/lease handling, and OpenJD adaptation.
- `renderacre`: PyO3/maturin extension module for Python submitters and pipeline tools.

## Control plane

The default implementation uses an in-memory scheduler so tests and demos stay
fast. Production-sized local farms can start the controller with the SQLite
backend, which persists the same scheduler state behind the existing REST and
Python contracts. The storage boundary is deliberately behind the scheduler API:
Postgres, object-backed logs, or managed/cloud control planes should replace
the backend without changing submitter or worker payloads.

Core endpoints:

- `POST /v1/jobs`: submit a direct task job or an OpenJD-backed job.
- `GET /v1/jobs/{job_id}`: inspect state and task output tails.
- `POST /v1/workers/register`: register a worker.
- `POST /v1/workers/{worker_id}/heartbeat`: keep worker online.
- `POST /v1/workers/{worker_id}/lease`: get the next runnable task.
- `POST /v1/tasks/{task_id}/started`: mark a leased task as running.
- `POST /v1/tasks/{task_id}/renew`: extend a healthy in-flight task lease.
- `POST /v1/tasks/{task_id}/complete`: report result and trigger retry/final state.

Task leases have a configurable controller-side TTL. Workers renew leases while
direct command and OpenJD tasks are still executing, so long-running work is not
dispatched twice while its worker remains healthy. If a worker disappears and no
renewal arrives before expiry, the scheduler recovers the task on the next lease
scan according to the retry path.

Worker capacity is enforced by the scheduler. A worker that registers multiple
slots can hold that many active leases, and `/v1/stats` reports total, used, and
available worker slots for API and dashboard consumers.

Task routing is schema-driven and backward-compatible. A task may require worker
labels and one of several pools; workers advertise those capabilities during
registration. Unconstrained tasks keep the previous behavior and can run on any
worker.

## OpenJD path

The controller accepts an OpenJD template bundle under `openjd.template_yaml`. It uses the official OpenJD Rust crates instead of a hand-written parser:

- `openjd-model` parses YAML/JSON, validates the template, preprocesses typed job parameters, creates the resolved job model, and expands step parameter spaces.
- Renderacre converts each resolved step/task combination into a farm task while preserving OpenJD runtime context such as parameter values, path mappings, environments, embedded files, and supported extensions.
- `openjd-sessions` runs the task on the worker, including environment enter/exit actions and OpenJD stdout directive handling.

OpenJD conversion stays separate from scheduler storage, so durable queues do not
change Python submitters or worker execution payloads.

## Python path

`crates/farm-python` builds the `renderacre` extension through maturin/PyO3. The wheel is configured as `cp37-abi3`, so one wheel per platform supports CPython 3.7 and newer.

The Python API is deliberately small:

- `command_job(...)` builds direct command payloads.
- `openjd_job(...)` builds OpenJD payloads from template text and JSON parameters.
- `submit_job(...)` posts a payload to a controller and returns the response JSON.

Keeping Python at the API edge lets DCC submitters stay pleasant while Rust owns scheduling, OpenJD validation, worker leases, and task execution.

## Deployment shape

The recommended first production shape is a private controller behind an
authenticated proxy plus one worker process per render node. OpenJD `PATH`
parameters should point at shared storage for scenes, scripts, and output
directories. The controller can run with in-memory storage for demos or SQLite
for a durable local farm.

For the lightweight durable profile, run:

```text
renderacre-controller --storage sqlite --sqlite-path /var/lib/renderacre/renderacre.sqlite3
```

SQLite persists submitted jobs, task attempts, leases, worker registrations, and
dashboard history needed for normal queue recovery. Expired leases are recovered
when workers resume leasing after a controller restart.

## Dashboard path

`dashboard/` is a Vite React application for queue operations. It reads `/v1/dashboard`, `/v1/jobs`, `/v1/workers`, and `/v1/stats`, then renders a Deadline-style queue table, worker assignment panel, OpenJD step detail, dependency mini graph, and stdout tail. During local development Vite proxies `/v1` to the controller; in deployment, serve the built static assets from `dashboard/dist` beside the controller API.

## Release assets

GitHub Releases build standalone `renderacre-controller` and `renderacre-worker` archives for Linux, macOS, and Windows. `scripts/install.sh` and `scripts/install.ps1` resolve the latest release by default and install both executables into a user-local bin directory.
