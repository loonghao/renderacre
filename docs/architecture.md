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
- `GET /healthz`, `GET /readyz`, `GET /v1/health`: inspect controller and scheduler readiness.
- `GET /v1/metrics`: read queue, worker, lease-renewal, and scheduler-error metrics.
- `GET /v1/audit`: read recent state-changing control-plane events.
- `POST /v1/workers/register`: register a worker.
- `POST /v1/workers/{worker_id}/heartbeat`: keep worker online.
- `POST /v1/workers/{worker_id}/lease`: get the next runnable task.
- `POST /v1/jobs/{job_id}/pause`: temporarily stop leasing pending work.
- `POST /v1/jobs/{job_id}/resume`: resume leasing a paused job.
- `POST /v1/jobs/{job_id}/cancel`: cancel remaining job work.
- `POST /v1/jobs/{job_id}/priority`: update queue priority.
- `POST /v1/tasks/{task_id}/cancel`: cancel a selected task.
- `POST /v1/tasks/{task_id}/requeue`: retry or requeue a selected task.
- `GET /v1/limits`: inspect shared resource and license usage.
- `POST /v1/limits`: define a named shared resource limit.
- `POST /v1/tasks/{task_id}/started`: mark a leased task as running.
- `POST /v1/tasks/{task_id}/renew`: extend a healthy in-flight task lease.
- `POST /v1/tasks/{task_id}/complete`: report result and trigger retry/final state.
- `GET /v1/tasks/{task_id}/attempts/{attempt_id}/logs`: read logs for one durable attempt record.
- `GET /v1/tasks/{task_id}/artifacts/{artifact_index}`: download a captured task artifact.

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
worker. OpenJD `hostRequirements` are carried through the same scheduler path:
standard amount capabilities are read from worker labels or slot capacity, and
standard attribute capabilities are read from worker labels.

Lifecycle actions are idempotent where repeating the same request is safe, and
invalid state transitions return a conflict response with a descriptive error.

Shared resource limits are named scheduler constraints for scarce farm-wide
capacity such as floating DCC licenses, GPU partitions, or heavy caches. Tasks
can request limits by name; undefined or exhausted limits block otherwise
runnable tasks until usage is released by completion, cancellation, requeue, or
lease expiry. Limit snapshots are exposed in the dashboard payload and the
dedicated `/v1/limits` API.

## Observability

Health endpoints return a machine-readable `HealthReport` with controller
readiness, scheduler backend name, scheduler readiness, and degraded component
names. Small farms can poll `/healthz`; deployment systems that separate
startup and readiness can use `/readyz` with the same payload.

`/v1/metrics` returns a `FarmMetrics` payload with queue depth, running tasks,
failed tasks, worker online/offline counts, lease renewal count, scheduler error
count, and the full queue `FarmStats` snapshot. `/v1/audit` returns recent
state-changing events with actor, action, target type/id, timestamp, outcome,
and message so operators can review queue mutations and worker transitions.

## OpenJD path

The controller accepts an OpenJD template bundle under `openjd.template_yaml`. It uses the official OpenJD Rust crates instead of a hand-written parser:

- `openjd-model` parses YAML/JSON, validates the template, preprocesses typed job parameters, creates the resolved job model, and expands step parameter spaces.
- Renderacre converts each resolved step/task combination into a farm task while preserving OpenJD runtime context such as parameter values, path mappings, environments, embedded files, supported extensions, dependencies, and host requirements.
- `openjd-sessions` runs the task on the worker, including environment enter/exit actions, OpenJD stdout directive handling, and progress/status callbacks.

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

## Logs and artifacts

The scheduler owns durable attempt metadata: each leased execution receives a
stable attempt id, worker identity, timing, exit code, concise failure summary,
stdout/stderr tails, artifact metadata, and a `log_ref` that resolves through
the controller API. Worker output is accepted as structured log batches and
attached to the active attempt when a task id is present, so running tasks can
show near-live stdout/stderr while completed attempts keep their own record.

The first artifact implementation records filesystem paths and serves readable
files through the controller. Workers can discover artifacts from submitted
artifact paths, modified files under output directories, and
`RENDERACRE_ARTIFACT=...` stdout/stderr directives. Future object storage
backends should preserve the same task artifact contract while replacing only
the path resolution and byte-serving implementation.

## Dashboard path

`dashboard/` is a Vite React application for queue operations. It reads `/v1/dashboard`, `/v1/jobs`, `/v1/workers`, and `/v1/stats`, then renders a Deadline-style queue table, worker assignment panel, OpenJD step detail, dependency mini graph, attempt history, artifacts, and stdout/stderr tails. During local development Vite proxies `/v1` to the controller; in deployment, serve the built static assets from `dashboard/dist` beside the controller API.

## Release assets

GitHub Releases build standalone `renderacre-controller` and `renderacre-worker` archives for Linux, macOS, and Windows. `scripts/install.sh` and `scripts/install.ps1` resolve the latest release by default and install both executables into a user-local bin directory.
