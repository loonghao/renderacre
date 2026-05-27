import { Monitor, RefreshCw } from "lucide-react";
import { formatDate, labelSummary } from "../lib/format";
import type { ApiWorker, FarmLog } from "../types";
import { LogStream, StateBadge } from "./Common";

export function WorkersView(props: {
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
