import {
  Activity,
  AlertCircle,
  Boxes,
  CheckCircle2,
  ChevronDown,
  Columns3,
  Cpu,
  FileCode2,
  Filter,
  LayoutList,
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
import { useEffect, useMemo, useState } from "react";

type JobState = "queued" | "running" | "succeeded" | "failed" | "cancelled";
type TaskState = "pending" | "leased" | "running" | "succeeded" | "failed" | "cancelled";

interface ApiTask {
  id: string;
  name: string;
  state: TaskState;
  attempts: number;
  max_retries: number;
  dependencies: string[];
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
  progress: number
): ApiJob {
  const succeeded = Math.floor((progress / 100) * 8);
  return {
    id,
    name,
    state,
    priority,
    created_at: "2026-05-27T09:12:44Z",
    updated_at: "2026-05-27T09:25:10Z",
    tasks: Array.from({ length: 8 }, (_, index) => ({
      id: `${id}-task-${index}`,
      name: `${step}_${String(index + 1).padStart(3, "0")}`,
      state:
        index < succeeded
          ? "succeeded"
          : state === "failed" && index === succeeded
            ? "failed"
            : state === "running" && index === succeeded
              ? "running"
              : "pending",
      attempts: index < succeeded ? 1 : 0,
      max_retries: 1,
      dependencies: [],
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
          : null,
      stderr_tail: state === "failed" ? "[ERROR] renderer returned exit code 1" : null
    }))
  };
}

export function App() {
  const [snapshot, setSnapshot] = useState<DashboardSnapshot>(sampleSnapshot);
  const [selectedJobId, setSelectedJobId] = useState(sampleSnapshot.jobs[0]?.id ?? "");
  const [query, setQuery] = useState("");
  const [live, setLive] = useState(true);
  const [source, setSource] = useState<"live" | "sample">("sample");

  async function loadDashboard() {
    try {
      const apiBase = import.meta.env.VITE_RENDERACRE_API_BASE ?? "";
      const response = await fetch(`${apiBase}/v1/dashboard`);
      if (!response.ok) throw new Error(`dashboard request failed: ${response.status}`);
      const data = (await response.json()) as DashboardSnapshot;
      setSnapshot(data.jobs.length ? data : sampleSnapshot);
      setSelectedJobId((current) => current || data.jobs[0]?.id || "");
      setSource("live");
    } catch {
      setSnapshot(sampleSnapshot);
      setSelectedJobId((current) => current || sampleSnapshot.jobs[0]?.id || "");
      setSource("sample");
    }
  }

  useEffect(() => {
    loadDashboard();
  }, []);

  useEffect(() => {
    if (!live) return;
    const timer = window.setInterval(loadDashboard, 5000);
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
  const selectedWorker = findWorker(snapshot.workers, selectedJob);

  return (
    <div className="app-shell">
      <Sidebar stats={snapshot.stats} source={source} />
      <main className="main">
        <Topbar query={query} setQuery={setQuery} live={live} setLive={setLive} onRefresh={loadDashboard} />
        <section className="metrics-grid">
          <Metric icon={<LayoutList />} label="Queued Tasks" value={snapshot.stats.tasks_pending} trend="+12.4%" tone="blue" />
          <Metric icon={<Play />} label="Running Tasks" value={snapshot.stats.tasks_running + snapshot.stats.tasks_leased} trend="+8.7%" tone="green" />
          <Metric icon={<CheckCircle2 />} label="Succeeded (24h)" value={snapshot.stats.tasks_succeeded} trend="+15.3%" tone="green" />
          <Metric icon={<XCircle />} label="Failed (24h)" value={snapshot.stats.tasks_failed} trend="-3.1%" tone="red" />
          <Metric icon={<Cpu />} label="Workers Online" value={`${snapshot.stats.workers_online} / ${snapshot.stats.workers_total}`} trend={`${workerPercent(snapshot.stats)}%`} tone="neutral" />
        </section>
        <section className="workspace">
          <QueueTable jobs={jobs} selectedJobId={selectedJob?.id} setSelectedJobId={setSelectedJobId} />
          {selectedJob ? <Inspector job={selectedJob} worker={selectedWorker} /> : <EmptyInspector />}
        </section>
      </main>
    </div>
  );
}

function Sidebar({ stats, source }: { stats: ApiStats; source: "live" | "sample" }) {
  const nav = [
    ["Queue", LayoutList],
    ["Workers", Monitor],
    ["OpenJD", FileCode2],
    ["Logs", TerminalSquare],
    ["Settings", Settings]
  ] as const;
  return (
    <aside className="sidebar">
      <div className="brand">
        <div className="brand-mark"><Boxes /></div>
        <strong>RENDER<span>ACRE</span></strong>
      </div>
      <nav className="nav">
        {nav.map(([label, Icon], index) => (
          <button className={index === 0 ? "nav-item active" : "nav-item"} key={label}>
            <Icon />
            {label}
          </button>
        ))}
      </nav>
      <div className="controller-card">
        <div><span className="status-dot online" /> Controller <small>v0.1.1</small></div>
        <dl>
          <dt>Jobs</dt><dd>{stats.jobs_total.toLocaleString()}</dd>
          <dt>Tasks</dt><dd>{stats.tasks_total.toLocaleString()}</dd>
          <dt>Workers</dt><dd>{stats.workers_online}/{stats.workers_total}</dd>
          <dt>API</dt><dd className={source === "live" ? "healthy" : "muted"}>{source === "live" ? "Live" : "Sample"}</dd>
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
  query: string;
  setQuery: (value: string) => void;
  live: boolean;
  setLive: (value: boolean) => void;
  onRefresh: () => void;
}) {
  return (
    <header className="topbar">
      <button className="icon-button"><LayoutList /></button>
      <h1>Queue</h1>
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

function Metric({ icon, label, value, trend, tone }: { icon: React.ReactNode; label: string; value: number | string; trend: string; tone: string }) {
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

function Inspector({ job, worker }: { job: ApiJob; worker?: ApiWorker }) {
  const runningTask = job.tasks.find((task) => task.state === "running" || task.state === "leased") ?? job.tasks[0];
  return (
    <aside className="inspector">
      <div className="inspector-head">
        <div><h2>{job.name}</h2><span>{job.id}</span></div>
        <button className="icon-button"><X /></button>
      </div>
      <div className="tabs"><button className="active">Overview</button><button>Tasks ({job.tasks.length})</button><button>Artifacts</button><button>Logs</button></div>
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
      <section className="terminal">
        <h3>Stdout Tail</h3>
        <pre>{runningTask?.stdout_tail ?? runningTask?.stderr_tail ?? "[INFO] Waiting for task output..."}</pre>
      </section>
    </aside>
  );
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
