import type { ReactNode } from "react";
import type { Node } from "@xyflow/react";

export type View = "queue" | "workers" | "openjd" | "logs" | "settings";
export type InspectorTab = "overview" | "workflow" | "tasks" | "artifacts" | "logs";
export type JobState = "queued" | "running" | "paused" | "succeeded" | "failed" | "cancelled";
export type TaskState = "pending" | "leased" | "running" | "succeeded" | "failed" | "cancelled";
export type JobAction = "pause" | "resume" | "cancel" | "priority";
export type TaskAction = "cancel" | "requeue";
export type LogLevel = "debug" | "info" | "warn" | "error";
export type LogSource = "controller" | "worker" | "task";

export interface ApiArtifact {
  name: string;
  path: string;
  size_bytes: number;
  kind: "image" | "scene" | "log" | "file";
  modified_at?: string | null;
}

export interface ApiTask {
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

export interface ApiJob {
  id: string;
  name: string;
  state: JobState;
  priority: number;
  created_at: string;
  updated_at: string;
  tasks: ApiTask[];
  openjd?: unknown;
}

export interface ApiWorker {
  id: string;
  name: string;
  labels: Record<string, string>;
  capacity: { slots: number };
  state: "online" | "offline";
  registered_at: string;
  last_seen_at: string;
}

export interface FarmLog {
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

export interface ApiStats {
  jobs_total: number;
  jobs_queued: number;
  jobs_running: number;
  jobs_paused: number;
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

export interface DashboardSnapshot {
  stats: ApiStats;
  jobs: ApiJob[];
  workers: ApiWorker[];
  logs: FarmLog[];
}

export interface WorkflowNode {
  task: ApiTask;
  layer: number;
  upstream: ApiTask[];
  downstream: ApiTask[];
  missingDependencies: string[];
}

export type FlowTaskNode = Node<{ label: ReactNode }>;
