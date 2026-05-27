import {
  Boxes,
  CheckCircle2,
  ChevronDown,
  Cpu,
  FileCode2,
  LayoutList,
  Monitor,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings,
  TerminalSquare,
  XCircle
} from "lucide-react";
import type { ReactNode } from "react";
import { workerPercent } from "../lib/jobs";
import type { ApiStats, View } from "../types";

export function Sidebar(props: {
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

export function Topbar(props: {
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

export function Metrics({ stats }: { stats: ApiStats }) {
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
