import type { ApiJob, DashboardSnapshot, JobState, LogLevel, LogSource, FarmLog } from "../types";

export const sampleSnapshot: DashboardSnapshot = {
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
  const taskIds = Array.from({ length: 8 }, (_, index) => `${id}-task-${index}`);
  const dependencyIndexes = [
    [],
    [0],
    [1],
    [1],
    [2, 3],
    [4],
    [4],
    [5, 6]
  ];
  return {
    id,
    name,
    state,
    priority,
    created_at: "2026-05-27T09:12:44Z",
    updated_at: "2026-05-27T09:25:10Z",
    tasks: taskIds.map((taskId, index) => {
      const done = index < succeeded;
      return {
        id: taskId,
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
        dependencies: dependencyIndexes[index].map((dependencyIndex) => taskIds[dependencyIndex]),
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
