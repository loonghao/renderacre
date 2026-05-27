import { useEffect, useMemo, useState } from "react";
import { EmptyInspector } from "./components/Common";
import { LogsView } from "./components/Logs";
import { Inspector, QueueTable } from "./components/Queue";
import { OpenJdView, SettingsView } from "./components/SecondaryViews";
import { Metrics, Sidebar, Topbar } from "./components/Shell";
import { WorkersView } from "./components/Workers";
import { sampleSnapshot } from "./data/sampleData";
import { findWorker, inferApplication, initialInspectorTab, initialView, normalizeSnapshot, viewTitle } from "./lib/jobs";
import type { DashboardSnapshot, InspectorTab, View } from "./types";

export function App() {
  const apiBase = import.meta.env.VITE_RENDERACRE_API_BASE ?? "";
  const [snapshot, setSnapshot] = useState<DashboardSnapshot>(sampleSnapshot);
  const [selectedJobId, setSelectedJobId] = useState(sampleSnapshot.jobs[0]?.id ?? "");
  const [selectedWorkerId, setSelectedWorkerId] = useState(sampleSnapshot.workers[0]?.id ?? "");
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>(initialInspectorTab);
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
