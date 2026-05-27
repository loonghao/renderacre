import { formatDate } from "../lib/format";
import type { ApiWorker, FarmLog } from "../types";

export function LogsView({ logs, workers }: { logs: FarmLog[]; workers: ApiWorker[] }) {
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
