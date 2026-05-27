import { sampleSnapshot } from "../data/sampleData";
import type { ApiJob, ApiStats, ApiWorker, DashboardSnapshot, InspectorTab, TaskState, View } from "../types";

export function inferApplication(job: ApiJob) {
  const task = job.tasks.find((candidate) => candidate.openjd?.task_parameters?.Application) ?? job.tasks[0];
  const value = task?.openjd?.task_parameters?.Application?.value;
  return typeof value === "string" ? value : job.openjd ? "OpenJD" : "Command";
}

export function inferStep(job: ApiJob) {
  return job.tasks.find((task) => task.openjd)?.openjd?.step_name ?? job.tasks[0]?.name ?? "main";
}

export function progress(job: ApiJob) {
  if (!job.tasks.length) return 0;
  const done = job.tasks.filter((task) => task.state === "succeeded").length;
  if (job.state === "failed") return Math.round((done / job.tasks.length) * 100);
  if (job.state === "succeeded") return 100;
  return Math.round((done / job.tasks.length) * 100);
}

export function leaseAge(job: ApiJob) {
  const leased = job.tasks.find((task) => task.lease);
  if (!leased?.lease) return "-";
  const elapsed = Math.max(0, Date.now() - Date.parse(leased.lease.leased_at));
  const seconds = Math.floor(elapsed / 1000);
  return `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
}

export function appIcon(app: string) {
  if (app.toLowerCase().includes("maya")) return "M";
  if (app.toLowerCase().includes("python")) return "Py";
  if (app.toLowerCase().includes("houdini")) return "H";
  if (app.toLowerCase().includes("nuke")) return "N";
  return "B";
}

export function workerPercent(stats: ApiStats) {
  return stats.workers_total ? Math.round((stats.workers_online / stats.workers_total) * 100) : 0;
}

export function findWorker(workers: ApiWorker[], job?: ApiJob) {
  const workerId = job?.tasks.find((task) => task.lease)?.lease?.worker_id;
  return workers.find((worker) => worker.id === workerId) ?? workers.find((worker) => worker.state === "online");
}

export function viewTitle(view: View) {
  return {
    queue: "Queue",
    workers: "Workers",
    openjd: "OpenJD",
    logs: "Logs",
    settings: "Settings"
  }[view];
}

export function initialView(): View {
  const value = window.location.hash.replace(/^#/, "");
  return isView(value) ? value : "queue";
}

export function initialInspectorTab(): InspectorTab {
  const value = new URLSearchParams(window.location.search).get("tab") ?? "";
  return isInspectorTab(value) ? value : "overview";
}

export function normalizeSnapshot(data: Partial<DashboardSnapshot>): DashboardSnapshot {
  return {
    stats: data.stats ?? sampleSnapshot.stats,
    jobs: data.jobs ?? [],
    workers: data.workers ?? [],
    logs: data.logs ?? [],
    limits: data.limits ?? []
  };
}

export function isView(value: string): value is View {
  return ["queue", "workers", "openjd", "logs", "settings"].includes(value);
}

function isInspectorTab(value: string): value is InspectorTab {
  return ["overview", "workflow", "tasks", "attempts", "artifacts", "logs"].includes(value);
}

export function tabLabel(tab: InspectorTab) {
  return {
    overview: "Overview",
    workflow: "Workflow",
    tasks: "Tasks",
    attempts: "Attempts",
    artifacts: "Artifacts",
    logs: "Logs"
  }[tab];
}

export function taskStateRank(state: TaskState) {
  return {
    running: 0,
    leased: 1,
    pending: 2,
    failed: 3,
    cancelled: 4,
    succeeded: 5
  }[state];
}
