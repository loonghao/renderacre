import { AlertCircle } from "lucide-react";
import type { ReactNode } from "react";
import { formatTime } from "../lib/format";
import type { FarmLog, JobState, TaskState } from "../types";

export function StateBadge({ state }: { state: JobState | TaskState }) {
  return <span className={`badge ${state}`}>{state}</span>;
}

export function Progress({ value }: { value: number }) {
  return <div className="progress"><span style={{ width: `${Math.min(100, Math.max(0, value))}%` }} /><em>{value}%</em></div>;
}

export function Terminal({ title, text }: { title: string; text: string }) {
  return (
    <section className="terminal">
      <h3>{title}</h3>
      <pre>{text}</pre>
    </section>
  );
}

export function LogStream({ logs, fallback }: { logs: FarmLog[]; fallback: string }) {
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

export function EmptyPanel({ icon, title }: { icon: ReactNode; title: string }) {
  return <section className="empty-panel">{icon}<strong>{title}</strong></section>;
}

export function EmptyInspector() {
  return <aside className="inspector empty"><AlertCircle /><h2>No job selected</h2></aside>;
}
