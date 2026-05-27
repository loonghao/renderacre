# Renderacre extension contracts

This note defines the contracts that let Renderacre grow from a local farm into
hybrid or cloud-backed deployments without changing submitter, worker, or
dashboard clients for every backend swap.

## Contract status

Stable extension points:

- The REST and Python job submission payloads, including OpenJD task conversion,
  task requirements, shared limits, artifact metadata, attempt records, and log
  references.
- Worker registration, heartbeat, lease, renewal, started, completion, and log
  batch payloads. Workers that do not send optional fields remain valid.
- The optional worker identity object shape: `provider`, `subject`,
  `attributes`, and `expires_at`.
- Scheduler decisions based on jobs, tasks, workers, leases, requirements, and
  resource limits rather than a concrete database or cloud provider.
- Health, readiness, metrics, audit, dashboard, log, and artifact read models as
  controller-facing operational contracts.

Experimental extension points:

- Postgres and managed control-plane storage implementations.
- Object storage log and artifact byte serving.
- Provider-specific worker identity validation, attestation, and credential
  refresh.
- Multi-controller scheduling coordination, leader election, and cloud autoscale
  policy hooks.

Experimental implementations must preserve stable payloads unless a documented
versioned contract replaces them.

## Storage backend responsibilities

Every storage backend owns durable scheduler state, not scheduling policy. It
must persist jobs, tasks, attempt records, worker registrations, leases, shared
limits, farm logs, audit events, and observability counters using the existing
domain models. It must restore enough state after restart for expired lease
recovery, retry accounting, dashboard history, and audit review.

The in-memory backend is for tests and demos only. It is allowed to lose all
state on process exit and should stay fast and dependency-free.

The SQLite backend is the lightweight durable profile. It stores the scheduler
snapshot locally, writes changes atomically, and keeps REST workers, dashboard
reads, and Python submitters on the same public contract used by the in-memory
backend.

A Postgres backend should provide the same scheduler state with transactional
writes, migrations, concurrent controller safety, and row-level or advisory
locking where needed. It must not add Postgres-specific fields to submitter or
worker payloads.

A managed or cloud backend should expose the same controller contract while
moving persistence, backups, and operational scaling behind the backend
boundary. Provider-specific metadata belongs in backend configuration or audit
events, not in core task scheduling decisions.

## Log and artifact storage responsibilities

The scheduler owns attempt metadata: attempt id, attempt number, worker id,
state, timing, exit code, failure summary, stdout/stderr tails, artifact list,
and `log_ref`. A log or artifact storage backend owns byte persistence and
retrieval behind those references.

The filesystem implementation records local artifact paths and serves readable
files through the controller. It must validate paths, avoid leaking unrelated
files, and preserve the `TaskArtifact` metadata returned by task completion.

An object storage implementation should store logs and artifacts under immutable
attempt-scoped keys, return controller-resolvable references or signed reads,
and preserve the same artifact metadata. It should not decide which worker gets
which task, which retry is allowed, or which resource limit is consumed.

## Worker identity and registration

Workers may register with an optional identity:

```json
{
  "name": "burst-worker-17",
  "labels": { "pool": "burst", "app": "blender" },
  "capacity": { "slots": 8 },
  "identity": {
    "provider": "aws-sts",
    "subject": "i-0123456789abcdef0",
    "attributes": { "region": "us-west-2" },
    "expires_at": "2026-05-28T10:30:00Z"
  }
}
```

Labels remain scheduling capabilities. Identity is provenance and authorization
context for provisioned or short-lived workers. Current scheduler decisions do
not inspect `identity`; future auth layers can reject expired or invalid
identities before registration, and future reconciliation can mark workers
offline when an identity expires without a fresh registration or heartbeat.

## Scheduler independence

The scheduler consumes normalized domain state: pending work, worker capacity,
worker labels, OpenJD host requirements, shared resource limits, leases, and
attempt history. It must not branch on storage engine, object store, region,
cloud account, or provider-specific identity fields.

Backends may decide how to persist, lock, replicate, and expose the state. They
must not change which task is runnable, which worker satisfies a requirement, or
how retries, leases, limits, pause, resume, cancel, and requeue transitions work.

## Extension rules

- Add optional fields with serde defaults when extending existing payloads.
- Keep provider-specific configuration out of stable task and worker contracts.
- Document any new stable field, endpoint, or state transition in the same PR
  that introduces it.
- Add focused tests for contract preservation across in-memory and durable
  backends when the change affects scheduler state.
- Prefer backend-specific adapters at the boundary over conditionals inside core
  scheduling rules.
