import { Columns3, Download, Filter, HardDrive, PauseCircle, Play, RotateCcw, SlidersHorizontal, X, XCircle } from "lucide-react";
import { formatBytes, formatDate } from "../lib/format";
import { appIcon, inferApplication, inferStep, leaseAge, progress, tabLabel } from "../lib/jobs";
import type { ApiJob, ApiTask, ApiWorker, FarmLog, InspectorTab, JobAction, TaskAction } from "../types";
import { EmptyPanel, LogStream, Progress, StateBadge, Terminal } from "./Common";
import { buildWorkflow, WorkflowPreview, WorkflowTab } from "./Workflow";

export function QueueTable({ jobs, selectedJobId, setSelectedJobId }: { jobs: ApiJob[]; selectedJobId?: string; setSelectedJobId: (id: string) => void }) {
  return (
    <section className="queue-panel">
      <div className="panel-header">
        <div><h2>Render Queue</h2><span>{jobs.length} jobs</span></div>
        <div className="panel-actions">
          <button className="button"><Filter />Filters</button>
          <button className="button"><Columns3 />Columns</button>
          <button className="icon-button"><SlidersHorizontal /></button>
        </div>
      </div>
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th></th>
              <th>Job Name</th>
              <th>Application</th>
              <th>OpenJD Step</th>
              <th>Progress</th>
              <th>Priority</th>
              <th>State</th>
              <th>Lease Age</th>
            </tr>
          </thead>
          <tbody>
            {jobs.map((job) => (
              <tr key={job.id} className={job.id === selectedJobId ? "selected" : ""} onClick={() => setSelectedJobId(job.id)}>
                <td><input type="checkbox" checked={job.id === selectedJobId} readOnly /></td>
                <td className="job-name">{job.name}</td>
                <td>{appIcon(inferApplication(job))} {inferApplication(job)}</td>
                <td>{inferStep(job)}</td>
                <td><Progress value={progress(job)} /></td>
                <td>{job.priority}</td>
                <td><StateBadge state={job.state} /></td>
                <td>{leaseAge(job)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <footer className="table-footer">Showing 1 to {jobs.length} of {jobs.length} jobs <span>20 / page</span></footer>
    </section>
  );
}

export function Inspector(props: {
  apiBase: string;
  job: ApiJob;
  worker?: ApiWorker;
  logs: FarmLog[];
  onJobAction: (action: JobAction, job: ApiJob) => void;
  onTaskAction: (action: TaskAction, task: ApiTask) => void;
  tab: InspectorTab;
  setTab: (tab: InspectorTab) => void;
}) {
  return (
    <aside className="inspector">
      <div className="inspector-head">
        <div><h2>{props.job.name}</h2><span>{props.job.id}</span></div>
        <button className="icon-button"><X /></button>
      </div>
      <div className="tabs">
        {(["overview", "workflow", "tasks", "artifacts", "logs"] as const).map((tab) => (
          <button className={props.tab === tab ? "active" : ""} key={tab} onClick={() => props.setTab(tab)}>
            {tabLabel(tab)}
          </button>
        ))}
      </div>
      {props.tab === "overview" ? <OverviewTab job={props.job} worker={props.worker} /> : null}
      {props.tab === "workflow" ? <WorkflowTab job={props.job} /> : null}
      {props.tab === "tasks" ? <TasksTab job={props.job} onTaskAction={props.onTaskAction} /> : null}
      {props.tab === "artifacts" ? <ArtifactsTab apiBase={props.apiBase} job={props.job} /> : null}
      {props.tab === "logs" ? <JobLogsTab job={props.job} logs={props.logs} /> : null}
      <JobActions job={props.job} onJobAction={props.onJobAction} />
    </aside>
  );
}

function OverviewTab({ job, worker }: { job: ApiJob; worker?: ApiWorker }) {
  const runningTask = job.tasks.find((task) => task.state === "running" || task.state === "leased") ?? job.tasks[0];
  return (
    <>
      <dl className="details">
        <dt>Application</dt><dd>{inferApplication(job)}</dd>
        <dt>OpenJD Step</dt><dd>{inferStep(job)}</dd>
        <dt>Priority</dt><dd>{job.priority}</dd>
        <dt>State</dt><dd><StateBadge state={job.state} /></dd>
        <dt>Progress</dt><dd><Progress value={progress(job)} /></dd>
        <dt>Created</dt><dd>{formatDate(job.created_at)}</dd>
        <dt>Updated</dt><dd>{formatDate(job.updated_at)}</dd>
      </dl>
      <WorkflowPreview job={job} />
      <WorkerAssignment worker={worker} />
      <Terminal title="Stdout Tail" text={runningTask?.stdout_tail ?? runningTask?.stderr_tail ?? "[INFO] Waiting for task output..."} />
    </>
  );
}

function TasksTab({ job, onTaskAction }: { job: ApiJob; onTaskAction: (action: TaskAction, task: ApiTask) => void }) {
  const workflow = buildWorkflow(job);
  return (
    <section className="task-list">
      {workflow.map(({ task, upstream, downstream, missingDependencies }) => (
        <article className="task-row" key={task.id}>
          <div>
            <strong>{task.name}</strong>
            <span>{task.id}</span>
          </div>
          <StateBadge state={task.state} />
          <small>attempt {task.attempts}/{Math.max(task.max_retries + 1, 1)}</small>
          <small>{upstream.length ? `${upstream.length} upstream` : "root"}</small>
          <small>{downstream.length ? `${downstream.length} downstream` : "terminal"}</small>
          <small>{requirementSummary(task)}</small>
          <small>{task.artifacts?.length ?? 0} artifacts</small>
          {missingDependencies.length ? <small className="dependency-warning">missing {missingDependencies.length}</small> : null}
          <div className="row-actions">
            <button className="icon-button" title="Requeue task" onClick={() => onTaskAction("requeue", task)}><RotateCcw /></button>
            <button className="icon-button danger" title="Cancel task" onClick={() => onTaskAction("cancel", task)}><XCircle /></button>
          </div>
        </article>
      ))}
    </section>
  );
}

function JobActions({ job, onJobAction }: { job: ApiJob; onJobAction: (action: JobAction, job: ApiJob) => void }) {
  return (
    <section className="inspector-actions">
      {job.state === "paused" ? (
        <button className="button" onClick={() => onJobAction("resume", job)}><Play />Resume</button>
      ) : (
        <button className="button" onClick={() => onJobAction("pause", job)}><PauseCircle />Pause</button>
      )}
      <button className="button" onClick={() => onJobAction("priority", job)}><SlidersHorizontal />Priority</button>
      <button className="button danger" onClick={() => onJobAction("cancel", job)}><XCircle />Cancel</button>
    </section>
  );
}

function ArtifactsTab({ apiBase, job }: { apiBase: string; job: ApiJob }) {
  const artifacts = job.tasks.flatMap((task) => (task.artifacts ?? []).map((artifact, index) => ({ task, artifact, index })));
  if (!artifacts.length) {
    return <EmptyPanel icon={<HardDrive />} title="No artifacts captured" />;
  }
  return (
    <section className="artifact-list">
      {artifacts.map(({ task, artifact, index }) => (
        <button className="artifact-row" key={`${task.id}-${index}`} onDoubleClick={() => openArtifact(apiBase, task.id, index)} onClick={() => openArtifact(apiBase, task.id, index)} title="Open artifact">
          <HardDrive />
          <div>
            <strong>{artifact.name}</strong>
            <span>{artifact.path}</span>
          </div>
          <small>{artifact.kind}</small>
          <small>{formatBytes(artifact.size_bytes)}</small>
          <Download />
        </button>
      ))}
    </section>
  );
}

function JobLogsTab({ job, logs }: { job: ApiJob; logs: FarmLog[] }) {
  const taskIds = new Set(job.tasks.map((task) => task.id));
  const jobLogs = logs.filter((log) => log.job_id === job.id || (log.task_id ? taskIds.has(log.task_id) : false));
  const tail = job.tasks
    .flatMap((task) => [task.stdout_tail, task.stderr_tail])
    .filter((value): value is string => Boolean(value?.trim()))
    .join("\n");
  return <LogStream logs={jobLogs} fallback={tail || "Waiting for job logs."} />;
}

function WorkerAssignment({ worker }: { worker?: ApiWorker }) {
  return (
    <section className="mini-card">
      <h3>Worker Assignment</h3>
      {worker ? (
        <dl className="details compact">
          <dt>Name</dt><dd><span className="status-dot online" />{worker.name}</dd>
          <dt>Pool</dt><dd>{worker.labels.pool ?? "default"}</dd>
          <dt>Capacity</dt><dd>{worker.capacity.slots} slots</dd>
          <dt>Last Seen</dt><dd>{formatDate(worker.last_seen_at)}</dd>
        </dl>
      ) : (
        <p className="muted">Waiting for a worker lease.</p>
      )}
    </section>
  );
}

function requirementSummary(task: ApiTask) {
  const requirements = task.requirements;
  if (!requirements) return "any worker";
  const labelCount = Object.keys(requirements.labels ?? {}).length;
  const poolCount = requirements.pools?.length ?? 0;
  const amountCount = requirements.amounts?.length ?? 0;
  const attributeCount = requirements.attributes?.length ?? 0;
  const total = labelCount + poolCount + amountCount + attributeCount;
  return total ? `${total} requirements` : "any worker";
}

function openArtifact(apiBase: string, taskId: string, artifactIndex: number) {
  window.open(`${apiBase}/v1/tasks/${taskId}/artifacts/${artifactIndex}`, "_blank", "noopener");
}
