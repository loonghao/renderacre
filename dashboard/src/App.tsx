import {
  Activity,
  AlertCircle,
  Boxes,
  CheckCircle2,
  ChevronDown,
  Columns3,
  Cpu,
  Download,
  FileCode2,
  Filter,
  HardDrive,
  LayoutList,
  ListChecks,
  Monitor,
  Play,
  Plus,
  RefreshCw,
  Search,
  Server,
  Settings,
  SlidersHorizontal,
  TerminalSquare,
  X,
  XCircle
} from "lucide-react";
import { useEffect, useMemo, useState, type ReactNode } from "react";

type View = "queue" | "workers" | "openjd" | "logs" | "settings";
type InspectorTab = "overview" | "tasks" | "artifacts" | "logs";
type JobState = "queued" | "running" | "succeeded" | "failed" | "cancelled";
type TaskState = "pending" | "leased" | "running" | "succeeded" | "failed" | "cancelled";
type LogLevel = "debug" | "info" | "warn" | "error";
type LogSource = "controller" | "worker" | "task";

interface ApiArtifact {
  name: string;
  path: string;
  size_bytes: number;
  kind: "image" | "scene" | "log" | "file";
  modified_at?: string | null;
}

interface ApiTask {
  id: string;
  name: string;
  state: TaskState;
  attempts: number;
  max_retries: number;
  dependencies: string[];
  last_exit_code?: number | null;
  artifact_paths?: string[];
  artifacts?: ApiArtifact[];
  lease?: {
    worker_id: string;
    leased_at: string;
    expires_at: string;
  } | null;
  openjd?: {
    step_name: string;
    task_parameters?: Record<string, { value: unknown }>;
  } | null;
  stdout_tail?: string | null;
  stderr_tail?: string | null;
}

interface ApiJob {
  id: string;
  name: string;
  state: JobState;
  priority: number;
  created_at: string;
  updated_at: string;
  tasks: ApiTask[];
  openjd?: unknown;
}

interface ApiWorker {
  id: string;
  name: string;
  labels: Record<string, string>;
  capacity: { slots: number };
  state: "online" | "offline";
  registered_at: string;
  last_seen_at: string;
}

interface FarmLog {
  id: string;
  timestamp: string;
  level: LogLevel;
  source: LogSource;
  stream?: string | null;
  message: string;
  job_id?: string | null;
  task_id?: string | null;
  worker_id?: string | null;
}

interface ApiStats {
  jobs_total: number;
  jobs_queued: number;
  jobs_running: number;
  jobs_succeeded: number;
  jobs_failed: number;
  tasks_total: number;
  tasks_pending: number;
  tasks_leased: number;
  tasks_running: number;
  tasks_succeeded: number;
  tasks_failed: number;
  workers_total: number;
  workers_online: number;
  workers_offline: number;
  worker_slots: number;
}

interface DashboardSnapshot {
  stats: ApiStats;
  jobs: ApiJob[];
  workers: ApiWorker[];
  logs: FarmLog[];
}

const sampleSnapshot: DashboardSnapshot = {
  stats: {
    jobs_total: 42,
    jobs_queued: 11,
    jobs_running: 14,
    jobs_succeeded: 14,
    jobs_failed: 3,
    tasks_total: 12842,
    tasks_pending: 312,
    tasks_leased: 38,
    tasks_running: 1128,
    tasks_succeeded: 10256,
    tasks_failed: 128,
    workers_total: 42,
    workers_online: 38,
    workers_offline: 4,
    worker_slots: 512
  },
  workers: [
    {
      id: "worker-07",
      name: "worker-07",
      labels: { pool: "3D-Windows", app: "blender,maya", gpu: "RTX 4090" },
      capacity: { slots: 32 },
      state: "online",
      registered_at: "2026-05-27T09:08:10Z",
      last_seen_at: "2026-05-27T09:25:10Z"
    },
    {
      id: "worker-12",
      name: "worker-12",
      labels: { pool: "Linux-Sim", app: "python,houdini" },
      capacity: { slots: 16 },
      state: "online",
      registered_at: "2026-05-27T09:08:10Z",
      last_seen_at: "2026-05-27T09:25:08Z"
    }
  ],
  logs: [
    sampleLog("log-01", "info", "controller", "job submitted: Shot_010_Lighting_v003", "job-01"),
    sampleLog("log-02", "info", "worker", "starting task Lighting_006", "job-01", "job-01-task-5", "worker-07", "worker"),
    sampleLog("log-03", "info", "worker", "[INFO] Rendering tiles on GPU", "job-01", "job-01-task-5", "worker-07", "stdout"),
    sampleLog("log-04", "error", "worker", "[ERROR] renderer returned exit code 1", "job-07", "job-07-task-0", "worker-12", "stderr")
  ],
  jobs: [
    sampleJob("job-01", "Shot_010_Lighting_v003", "running", 75, "Blender 4.1", "Lighting", 78),
    sampleJob("job-02", "Shot_010_Comp_v002", "running", 50, "Nuke 15.2", "Comp", 42),
    sampleJob("job-03", "Shot_020_Anim_v004", "running", 60, "Maya 2024", "Animation", 15),
    sampleJob("job-04", "Shot_020_Sim_v001", "queued", 60, "Houdini 20.5", "Sim", 0),
    sampleJob("job-05", "Asset_Robot_Render", "succeeded", 50, "Maya 2024", "Render", 100),
    sampleJob("job-06", "Util_Python_Tools", "succeeded", 25, "Python 3.11", "Process", 100),
    sampleJob("job-07", "Shot_040_Lighting_v001", "failed", 75, "Blender 4.1", "Lighting", 0)
  ]
};

function sampleJob(
  id: string,
  name: string,
  state: JobState,
  priority: number,
  app: string,
  step: string,
  progressValue: number
): ApiJob {
  const succeeded = Math.floor((progressValue / 100) * 8);
  return {
    id,
    name,
    state,
    priority,
    created_at: "2026-05-27T09:12:44Z",
    updated_at: "2026-05-27T09:25:10Z",
    tasks: Array.from({ length: 8 }, (_, index) => {
      const done = index < succeeded;
      return {
        id: `${id}-task-${index}`,
        name: `${step}_${String(index + 1).padStart(3, "0")}`,
        state:
          done
            ? "succeeded"
            : state === "failed" && index === succeeded
              ? "failed"
              : state === "running" && index === succeeded
                ? "running"
                : "pending",
        attempts: done ? 1 : 0,
        max_retries: 1,
        dependencies: [],
        last_exit_code: done ? 0 : null,
        artifact_paths: ["//show/render/shot010"],
        artifacts: done
          ? [
              {
                name: `${name}_${1001 + index}.png`,
                path: `//show/render/${name}_${1001 + index}.png`,
                size_bytes: 182044,
                kind: "image",
                modified_at: "2026-05-27T09:24:10Z"
              }
            ]
          : [],
        lease: state === "running" && index === succeeded ? { worker_id: "worker-07", leased_at: "2026-05-27T09:18:01Z", expires_at: "2026-05-27T09:28:01Z" } : null,
        openjd: {
          step_name: step,
          task_parameters: {
            Application: { value: app },
            Frame: { value: `${1001 + index}` }
          }
        },
        stdout_tail:
          state === "running"
            ? "[INFO] Starting lighting pass\n[INFO] Scene loaded\n[INFO] Frame 1064/1128 (55.8%)\n[INFO] Rendering tiles on GPU"
            : done
              ? "RENDERACRE_ARTIFACT=//show/render/frame.png\n[INFO] Render complete"
              : null,
        stderr_tail: state === "failed" ? "[ERROR] renderer returned exit code 1" : null
      };
    })
  };
}

function sampleLog(
  id: string,
  level: LogLevel,
  source: LogSource,
  message: string,
  jobId?: string,
  taskId?: string,
  workerId?: string,
  stream?: string
): FarmLog {
  return {
    id,
    timestamp: "2026-05-27T09:25:10Z",
    level,
    source,
    stream,
    message,
    job_id: jobId,
    task_id: taskId,
    worker_id: workerId
  };
}

export function App() {
  const apiBase = import.meta.env.VITE_RENDERACRE_API_BASE ?? "";
  const [snapshot, setSnapshot] = useState<DashboardSnapshot>(sampleSnapshot);
  const [selectedJobId, setSelectedJobId] = useState(sampleSnapshot.jobs[0]?.id ?? "");
  const [selectedWorkerId, setSelectedWorkerId] = useState(sampleSnapshot.workers[0]?.id ?? "");
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("overview");
  const [activeView, setActiveViewState] = useState<View>(initialView);
  const [query, setQuery] = useState("");
  const [live, setLive] = useState(true);
  const [source, setSource] = useState<"live" | "sample">("sample");

  async function loadDashboard() {
    try {
      const response = await fetch(`${apiBase}/v1/dashboard`);
      if (!response.ok) throw new Error(`dashboard request failed: ${response.status}`);
      const data = (await response.json()) as DashboardSnapshot;
      const normalized = normalizeSnapshot(data);
      const next = normalized.jobs.length || normalized.workers.length ? normalized : sampleSnapshot;
      setSnapshot(next);
      setSelectedJobId((current) => current || next.jobs[0]?.id || "");
      setSelectedWorkerId((current) => current || next.workers[0]?.id || "");
      setSource("live");
    } catch {
      setSnapshot(sampleSnapshot);
      setSelectedJobId((current) => current || sampleSnapshot.jobs[0]?.id || "");
      setSelectedWorkerId((current) => current || sampleSnapshot.workers[0]?.id || "");
      setSource("sample");
    }
  }

  useEffect(() => {
    loadDashboard();
  }, []);

  useEffect(() => {
    const onHashChange = () => setActiveViewState(initialView());
    window.addEventListener("hashchange", onHashChange);
    return () => window.removeEventListener("hashchange", onHashChange);
  }, []);

  function setActiveView(view: View) {
    setActiveViewState(view);
    window.location.hash = view === "queue" ? "" : view;
  }

  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(loadDashboard, 2000);
    return () => window.clearInterval(timer);
  }, [live]);

  const jobs = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return snapshot.jobs;
    return snapshot.jobs.filter((job) => {
      const app = inferApplication(job).toLowerCase();
      return job.name.toLowerCase().includes(needle) || app.includes(needle) || job.state.includes(needle);
    });
  }, [query, snapshot.jobs]);

  const selectedJob = jobs.find((job) => job.id === selectedJobId) ?? jobs[0] ?? snapshot.jobs[0];
  const selectedWorker = snapshot.workers.find((worker) => worker.id === selectedWorkerId) ?? snapshot.workers[0];

  return (
    <div className="app-shell">
      <Sidebar stats={snapshot.stats} source={source} activeView={activeView} setActiveView={setActiveView} />
      <main className="main">
        <Topbar title={viewTitle(activeView)} query={query} setQuery={setQuery} live={live} setLive={setLive} onRefresh={loadDashboard} />
        {activeView === "queue" ? (
          <>
            <Metrics stats={snapshot.stats} />
            <section className="workspace">
              <QueueTable jobs={jobs} selectedJobId={selectedJob?.id} setSelectedJobId={setSelectedJobId} />
              {selectedJob ? (
                <Inspector
                  apiBase={apiBase}
                  job={selectedJob}
                  logs={snapshot.logs}
                  worker={findWorker(snapshot.workers, selectedJob)}
                  tab={inspectorTab}
                  setTab={setInspectorTab}
                />
              ) : (
                <EmptyInspector />
              )}
            </section>
          </>
        ) : null}
        {activeView === "workers" ? (
          <WorkersView
            logs={snapshot.logs}
            selectedWorker={selectedWorker}
            selectedWorkerId={selectedWorkerId}
            setSelectedWorkerId={setSelectedWorkerId}
            workers={snapshot.workers}
          />
        ) : null}
        {activeView === "logs" ? <LogsView logs={snapshot.logs} workers={snapshot.workers} /> : null}
        {activeView === "openjd" ? <OpenJdView jobs={snapshot.jobs} /> : null}
        {activeView === "settings" ? <SettingsView source={source} stats={snapshot.stats} /> : null}
      </main>
    </div>
  );
}

function Sidebar(props: {
  stats: ApiStats;
  source: "live" | "sample";
  activeView: View;
  setActiveView: (view: View) => void;
}) {
  const nav = [
    ["queue", "Queue", LayoutList],
    ["workers", "Workers", Monitor],
    ["openjd", "OpenJD", FileCode2],
    ["logs", "Logs", TerminalSquare],
    ["settings", "Settings", Settings]
  ] as const;
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark"><Boxes /></div>
        <strong>RENDER<span>ACRE</span></strong>
      </div>
      <nav className="nav">
        {nav.map(([view, label, Icon]) => (
          <button className={props.activeView === view ? "nav-item active" : "nav-item"} key={view} onClick={() => props.setActiveView(view)}>
            <Icon />
            {label}
          </button>
        ))}
      </nav>
      <div className="controller-card">
        <div><span className="status-dot online" /> Controller <small>v0.1.4</small></div>
        <dl>
          <dt>Jobs</dt><dd>{props.stats.jobs_total.toLocaleString()}</dd>
          <dt>Tasks</dt><dd>{props.stats.tasks_total.toLocaleString()}</dd>
          <dt>Workers</dt><dd>{props.stats.workers_online}/{props.stats.workers_total}</dd>
          <dt>API</dt><dd className={props.source === "live" ? "healthy" : "muted"}>{props.source === "live" ? "Live" : "Sample"}</dd>
        </dl>
      </div>
      <div className="user-card">
        <div className="avatar">A</div>
        <div><strong>admin</strong><span>Administrator</span></div>
        <ChevronDown />
      </div>
    </aside>
  );
}

function Topbar(props: {
  title: string;
  query: string;
  setQuery: (value: string) => void;
  live: boolean;
  setLive: (value: boolean) => void;
  onRefresh: () => void;
}) {
  return (
    <header className="topbar">
      <button className="icon-button"><LayoutList /></button>
      <h1>{props.title}</h1>
      <label className="search">
        <Search />
        <input value={props.query} onChange={(event) => props.setQuery(event.target.value)} placeholder="Search jobs, tasks, workers..." />
        <kbd>/</kbd>
      </label>
      <button className="button primary"><Plus />Submit Job</button>
      <button className="button" onClick={props.onRefresh}><RefreshCw />Refresh</button>
      <button className={props.live ? "button live active" : "button live"} onClick={() => props.setLive(!props.live)}>
        <span className="status-dot online" />Live
      </button>
    </header>
  );
}

function Metrics({ stats }: { stats: ApiStats }) {
  return (
    <section className="metrics-grid">
      <Metric icon={<LayoutList />} label="Queued Tasks" value={stats.tasks_pending} trend="+12.4%" tone="blue" />
      <Metric icon={<Play />} label="Running Tasks" value={stats.tasks_running + stats.tasks_leased} trend="+8.7%" tone="green" />
      <Metric icon={<CheckCircle2 />} label="Succeeded (24h)" value={stats.tasks_succeeded} trend="+15.3%" tone="green" />
      <Metric icon={<XCircle />} label="Failed (24h)" value={stats.tasks_failed} trend="-3.1%" tone="red" />
      <Metric icon={<Cpu />} label="Workers Online" value={`${stats.workers_online} / ${stats.workers_total}`} trend={`${workerPercent(stats)}%`} tone="neutral" />
    </section>
  );
}

function Metric({ icon, label, value, trend, tone }: { icon: ReactNode; label: string; value: number | string; trend: string; tone: string }) {
  return (
    <article className="metric-card">
      <div className={`metric-icon ${tone}`}>{icon}</div>
      <span>{label}</span>
      <strong>{typeof value === "number" ? value.toLocaleString() : value}</strong>
      <small className={trend.startsWith("-") ? "negative" : "positive"}>{trend} vs 24h ago</small>
    </article>
  );
}

function QueueTable({ jobs, selectedJobId, setSelectedJobId }: { jobs: ApiJob[]; selectedJobId?: string; setSelectedJobId: (id: string) => void }) {
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

function Inspector(props: {
  apiBase: string;
  job: ApiJob;
  worker?: ApiWorker;
  logs: FarmLog[];
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
        {(["overview", "tasks", "artifacts", "logs"] as const).map((tab) => (
          <button className={props.tab === tab ? "active" : ""} key={tab} onClick={() => props.setTab(tab)}>
            {tabLabel(tab)}
          </button>
        ))}
      </div>
      {props.tab === "overview" ? <OverviewTab job={props.job} worker={props.worker} /> : null}
      {props.tab === "tasks" ? <TasksTab job={props.job} /> : null}
      {props.tab === "artifacts" ? <ArtifactsTab apiBase={props.apiBase} job={props.job} /> : null}
      {props.tab === "logs" ? <JobLogsTab job={props.job} logs={props.logs} /> : null}
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
      <section className="mini-card">
        <h3>Task Dependency Graph</h3>
        <div className="graph">
          {["Prepare", "Cache", inferStep(job), "Publish"].map((label, index) => (
            <div className={index === 2 ? "node active" : "node"} key={label}>
              <strong>{label}</strong>
              <span>{index < 2 ? "done" : index === 2 ? `${progress(job)}%` : "waiting"}</span>
            </div>
          ))}
        </div>
      </section>
      <WorkerAssignment worker={worker} />
      <Terminal title="Stdout Tail" text={runningTask?.stdout_tail ?? runningTask?.stderr_tail ?? "[INFO] Waiting for task output..."} />
    </>
  );
}

function TasksTab({ job }: { job: ApiJob }) {
  return (
    <section className="task-list">
      {job.tasks.map((task) => (
        <article className="task-row" key={task.id}>
          <div>
            <strong>{task.name}</strong>
            <span>{task.id}</span>
          </div>
          <StateBadge state={task.state} />
          <small>attempt {task.attempts}/{Math.max(task.max_retries + 1, 1)}</small>
          <small>{task.artifacts?.length ?? 0} artifacts</small>
        </article>
      ))}
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

function WorkersView(props: {
  workers: ApiWorker[];
  logs: FarmLog[];
  selectedWorker?: ApiWorker;
  selectedWorkerId: string;
  setSelectedWorkerId: (id: string) => void;
}) {
  const workerLogs = props.logs.filter((log) => log.worker_id === props.selectedWorker?.id);
  return (
    <section className="workers-layout">
      <section className="queue-panel">
        <div className="panel-header">
          <div><h2>Workers</h2><span>{props.workers.length} nodes</span></div>
          <div className="panel-actions"><button className="button"><RefreshCw />Ping</button></div>
        </div>
        <div className="worker-grid">
          {props.workers.map((worker) => (
            <button className={worker.id === props.selectedWorkerId ? "worker-card selected" : "worker-card"} key={worker.id} onClick={() => props.setSelectedWorkerId(worker.id)}>
              <div><Monitor /><strong>{worker.name}</strong><StateBadge state={worker.state === "online" ? "running" : "failed"} /></div>
              <dl>
                <dt>Slots</dt><dd>{worker.capacity.slots}</dd>
                <dt>Last Seen</dt><dd>{formatDate(worker.last_seen_at)}</dd>
                <dt>Labels</dt><dd>{labelSummary(worker.labels)}</dd>
              </dl>
            </button>
          ))}
        </div>
      </section>
      <section className="worker-log-panel">
        <div className="panel-header compact-header">
          <div><h2>{props.selectedWorker?.name ?? "Worker"} Logs</h2><span>{workerLogs.length} lines</span></div>
        </div>
        <LogStream logs={workerLogs} fallback="Waiting for worker logs." />
      </section>
    </section>
  );
}

function LogsView({ logs, workers }: { logs: FarmLog[]; workers: ApiWorker[] }) {
  const names = new Map(workers.map((worker) => [worker.id, worker.name]));
  return (
    <section className="logs-page">
      <section className="queue-panel">
        <div className="panel-header"><div><h2>Farm Logs</h2><span>{logs.length} events</span></div></div>
        <div className="log-table">
          {logs.map((log) => (
            <article className={`log-row ${log.level}`} key={log.id}>
              <time>{formatDate(log.timestamp)}</time>
              <span>{log.level}</span>
              <span>{log.worker_id ? names.get(log.worker_id) ?? log.worker_id : log.source}</span>
              <code>{log.stream ?? log.source}</code>
              <p>{log.message}</p>
            </article>
          ))}
        </div>
      </section>
    </section>
  );
}

function OpenJdView({ jobs }: { jobs: ApiJob[] }) {
  const openjdJobs = jobs.filter((job) => job.openjd || job.tasks.some((task) => task.openjd));
  return (
    <section className="logs-page">
      <section className="queue-panel">
        <div className="panel-header"><div><h2>OpenJD Jobs</h2><span>{openjdJobs.length} jobs</span></div></div>
        <div className="task-list">
          {openjdJobs.map((job) => (
            <article className="task-row" key={job.id}>
              <div>
                <strong>{job.name}</strong>
                <span>{inferStep(job)} / {inferApplication(job)}</span>
              </div>
              <Progress value={progress(job)} />
              <StateBadge state={job.state} />
            </article>
          ))}
        </div>
      </section>
    </section>
  );
}

function SettingsView({ source, stats }: { source: "live" | "sample"; stats: ApiStats }) {
  return (
    <section className="logs-page">
      <section className="settings-grid">
        <article className="mini-card settings-card"><Server /><strong>Controller</strong><span>{source === "live" ? "Live API" : "Sample data"}</span></article>
        <article className="mini-card settings-card"><Activity /><strong>Workers</strong><span>{stats.workers_online}/{stats.workers_total} online</span></article>
        <article className="mini-card settings-card"><ListChecks /><strong>Tasks</strong><span>{stats.tasks_running + stats.tasks_leased} active</span></article>
      </section>
    </section>
  );
}

function LogStream({ logs, fallback }: { logs: FarmLog[]; fallback: string }) {
  if (!logs.length) return <Terminal title="Live Log" text={fallback} />;
  return (
    <section className="terminal log-stream">
      <h3>Live Log</h3>
      <div>
        {logs.slice(-120).map((log) => (
          <p className={log.level} key={log.id}>
            <time>{formatTime(log.timestamp)}</time>
            <span>{log.stream ?? log.source}</span>
            {log.message}
          </p>
        ))}
      </div>
    </section>
  );
}

function Terminal({ title, text }: { title: string; text: string }) {
  return (
    <section className="terminal">
      <h3>{title}</h3>
      <pre>{text}</pre>
    </section>
  );
}

function EmptyPanel({ icon, title }: { icon: ReactNode; title: string }) {
  return <section className="empty-panel">{icon}<strong>{title}</strong></section>;
}

function EmptyInspector() {
  return <aside className="inspector empty"><AlertCircle /><h2>No job selected</h2></aside>;
}

function StateBadge({ state }: { state: JobState | TaskState }) {
  return <span className={`badge ${state}`}>{state}</span>;
}

function Progress({ value }: { value: number }) {
  return <div className="progress"><span style={{ width: `${Math.min(100, Math.max(0, value))}%` }} /><em>{value}%</em></div>;
}

function inferApplication(job: ApiJob) {
  const task = job.tasks.find((candidate) => candidate.openjd?.task_parameters?.Application) ?? job.tasks[0];
  const value = task?.openjd?.task_parameters?.Application?.value;
  return typeof value === "string" ? value : job.openjd ? "OpenJD" : "Command";
}

function inferStep(job: ApiJob) {
  return job.tasks.find((task) => task.openjd)?.openjd?.step_name ?? job.tasks[0]?.name ?? "main";
}

function progress(job: ApiJob) {
  if (!job.tasks.length) return 0;
  const done = job.tasks.filter((task) => task.state === "succeeded").length;
  if (job.state === "failed") return Math.round((done / job.tasks.length) * 100);
  if (job.state === "succeeded") return 100;
  return Math.round((done / job.tasks.length) * 100);
}

function leaseAge(job: ApiJob) {
  const leased = job.tasks.find((task) => task.lease);
  if (!leased?.lease) return "-";
  const elapsed = Math.max(0, Date.now() - Date.parse(leased.lease.leased_at));
  const seconds = Math.floor(elapsed / 1000);
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

function appIcon(app: string) {
  if (app.toLowerCase().includes("maya")) return "M";
  if (app.toLowerCase().includes("python")) return "Py";
  if (app.toLowerCase().includes("houdini")) return "H";
  if (app.toLowerCase().includes("nuke")) return "N";
  return "B";
}

function workerPercent(stats: ApiStats) {
  return stats.workers_total ? Math.round((stats.workers_online / stats.workers_total) * 100) : 0;
}

function findWorker(workers: ApiWorker[], job?: ApiJob) {
  const workerId = job?.tasks.find((task) => task.lease)?.lease?.worker_id;
  return workers.find((worker) => worker.id === workerId) ?? workers.find((worker) => worker.state === "online");
}

function formatDate(value: string) {
  return new Intl.DateTimeFormat(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}

function formatTime(value: string) {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit"
  }).format(new Date(value));
}

function formatBytes(value: number) {
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MB`;
}

function labelSummary(labels: Record<string, string>) {
  const values = Object.entries(labels).map(([key, value]) => `${key}=${value}`);
  return values.length ? values.join(", ") : "default";
}

function viewTitle(view: View) {
  return {
    queue: "Queue",
    workers: "Workers",
    openjd: "OpenJD",
    logs: "Logs",
    settings: "Settings"
  }[view];
}

function initialView(): View {
  const value = window.location.hash.replace(/^#/, "");
  return isView(value) ? value : "queue";
}

function normalizeSnapshot(data: Partial<DashboardSnapshot>): DashboardSnapshot {
  return {
    stats: data.stats ?? sampleSnapshot.stats,
    jobs: data.jobs ?? [],
    workers: data.workers ?? [],
    logs: data.logs ?? []
  };
}

function isView(value: string): value is View {
  return ["queue", "workers", "openjd", "logs", "settings"].includes(value);
}

function tabLabel(tab: InspectorTab) {
  return {
    overview: "Overview",
    tasks: "Tasks",
    artifacts: "Artifacts",
    logs: "Logs"
  }[tab];
}

function openArtifact(apiBase: string, taskId: string, artifactIndex: number) {
  window.open(`${apiBase}/v1/tasks/${taskId}/artifacts/${artifactIndex}`, "_blank", "noopener");
}
