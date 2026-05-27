import { Activity, ListChecks, Server } from "lucide-react";
import { inferApplication, inferStep, progress } from "../lib/jobs";
import type { ApiJob, ApiLimit, ApiStats } from "../types";
import { Progress, StateBadge } from "./Common";

export function OpenJdView({ jobs }: { jobs: ApiJob[] }) {
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

export function SettingsView({ limits, source, stats }: { limits: ApiLimit[]; source: "live" | "sample"; stats: ApiStats }) {
  return (
    <section className="logs-page">
      <section className="settings-grid">
        <article className="mini-card settings-card"><Server /><strong>Controller</strong><span>{source === "live" ? "Live API" : "Sample data"}</span></article>
        <article className="mini-card settings-card"><Activity /><strong>Workers</strong><span>{stats.workers_online}/{stats.workers_total} online</span></article>
        <article className="mini-card settings-card"><ListChecks /><strong>Tasks</strong><span>{stats.tasks_running + stats.tasks_leased} active</span></article>
      </section>
      <section className="queue-panel">
        <div className="panel-header"><div><h2>Shared Limits</h2><span>{limits.length} limits</span></div></div>
        <div className="task-list">
          {limits.map((limit) => (
            <article className="task-row" key={limit.name}>
              <div>
                <strong>{limit.name}</strong>
                <span>{limit.used}/{limit.max_count} in use</span>
              </div>
              <Progress value={limit.max_count ? Math.round((limit.used / limit.max_count) * 100) : 0} />
              <small>{limit.available} available</small>
            </article>
          ))}
        </div>
      </section>
    </section>
  );
}
