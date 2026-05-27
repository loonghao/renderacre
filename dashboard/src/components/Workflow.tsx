import {
  Background,
  Controls,
  MarkerType,
  Position,
  ReactFlow,
  type Edge
} from "@xyflow/react";
import { useMemo } from "react";
import { taskStateRank } from "../lib/jobs";
import type { ApiJob, ApiTask, FlowTaskNode, WorkflowNode } from "../types";
import { StateBadge } from "./Common";
import "@xyflow/react/dist/style.css";

export function WorkflowPreview({ job }: { job: ApiJob }) {
  const workflow = buildWorkflow(job);
  const layers = groupWorkflowLayers(workflow);
  const edgeCount = workflow.reduce((count, node) => count + node.downstream.length, 0);
  return (
    <section className="mini-card">
      <div className="mini-card-title">
        <h3>Task Dependency Graph</h3>
        <span>{edgeCount} edges</span>
      </div>
      <div className="graph">
        {layers.map((layer, index) => (
          <div className="node" key={index}>
            <strong>Stage {index + 1}</strong>
            <span>{layer.length} task{layer.length === 1 ? "" : "s"}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

export function WorkflowTab({ job }: { job: ApiJob }) {
  const flow = useMemo(() => buildWorkflowFlow(job), [job]);
  const workflow = flow.workflow;
  const edgeCount = workflow.reduce((count, node) => count + node.downstream.length, 0);
  const blockedCount = workflow.filter((node) => isBlockedByUpstream(node)).length;
  return (
    <section className="workflow-panel">
      <div className="workflow-summary">
        <SummaryPill label="Tasks" value={workflow.length} />
        <SummaryPill label="Depends" value={edgeCount} />
        <SummaryPill label="Ready" value={workflow.filter((node) => !isBlockedByUpstream(node)).length} />
        <SummaryPill label="Blocked" value={blockedCount} />
      </div>
      <div className="workflow-flow" style={{ height: flow.height }}>
        <ReactFlow
          key={job.id}
          nodes={flow.nodes}
          edges={flow.edges}
          fitView
          fitViewOptions={{ padding: 0.16 }}
          nodesDraggable={false}
          nodesConnectable={false}
          elementsSelectable
          minZoom={0.35}
          maxZoom={1.4}
        >
          <Background color="#cad5e2" gap={18} />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>
      <div className="workflow-legend">
        <span><i className="legend-dot running" />active</span>
        <span><i className="legend-dot succeeded" />done</span>
        <span><i className="legend-dot pending" />waiting</span>
        <span><i className="legend-dot failed" />failed</span>
      </div>
    </section>
  );
}

export function buildWorkflow(job: ApiJob): WorkflowNode[] {
  const taskById = new Map(job.tasks.map((task) => [task.id, task]));
  const downstreamById = new Map<string, ApiTask[]>();
  const missingById = new Map<string, string[]>();

  for (const task of job.tasks) {
    for (const dependencyId of task.dependencies ?? []) {
      const dependency = taskById.get(dependencyId);
      if (!dependency) {
        const missing = missingById.get(task.id) ?? [];
        missing.push(dependencyId);
        missingById.set(task.id, missing);
        continue;
      }
      const downstream = downstreamById.get(dependency.id) ?? [];
      downstream.push(task);
      downstreamById.set(dependency.id, downstream);
    }
  }

  const memo = new Map<string, number>();
  const visiting = new Set<string>();
  const layerFor = (task: ApiTask): number => {
    const cached = memo.get(task.id);
    if (cached !== undefined) return cached;
    if (visiting.has(task.id)) return 0;
    visiting.add(task.id);
    const upstreamLayers = (task.dependencies ?? [])
      .map((dependencyId) => taskById.get(dependencyId))
      .filter((dependency): dependency is ApiTask => Boolean(dependency))
      .map((dependency) => layerFor(dependency));
    visiting.delete(task.id);
    const layer = upstreamLayers.length ? Math.max(...upstreamLayers) + 1 : 0;
    memo.set(task.id, layer);
    return layer;
  };

  return job.tasks
    .map((task) => ({
      task,
      layer: layerFor(task),
      upstream: (task.dependencies ?? [])
        .map((dependencyId) => taskById.get(dependencyId))
        .filter((dependency): dependency is ApiTask => Boolean(dependency)),
      downstream: downstreamById.get(task.id) ?? [],
      missingDependencies: missingById.get(task.id) ?? []
    }))
    .sort((left, right) => left.layer - right.layer || taskStateRank(left.task.state) - taskStateRank(right.task.state) || left.task.name.localeCompare(right.task.name));
}

export function groupWorkflowLayers(nodes: WorkflowNode[]) {
  const layers: WorkflowNode[][] = [];
  for (const node of nodes) {
    const layer = layers[node.layer] ?? [];
    layer.push(node);
    layers[node.layer] = layer;
  }
  return layers.length ? layers : [[]];
}

export function isBlockedByUpstream(node: WorkflowNode) {
  return node.missingDependencies.length > 0 || node.upstream.some((task) => task.state !== "succeeded");
}

function buildWorkflowFlow(job: ApiJob): {
  workflow: WorkflowNode[];
  nodes: FlowTaskNode[];
  edges: Edge[];
  height: number;
} {
  const workflow = buildWorkflow(job);
  const layers = groupWorkflowLayers(workflow);
  const laneByTaskId = new Map<string, number>();
  for (const layer of layers) {
    layer.forEach((node, index) => laneByTaskId.set(node.task.id, index));
  }

  const nodes = workflow.map((node) => ({
    id: node.task.id,
    position: {
      x: (laneByTaskId.get(node.task.id) ?? 0) * 260,
      y: node.layer * 150
    },
    sourcePosition: Position.Bottom,
    targetPosition: Position.Top,
    data: {
      label: <WorkflowTaskLabel node={node} />
    },
    className: `workflow-flow-node ${node.task.state} ${isBlockedByUpstream(node) ? "blocked" : ""}`,
    draggable: false
  }));

  const edges = workflow.flatMap((node) =>
    node.downstream.map((target) => ({
      id: `${node.task.id}-${target.id}`,
      source: node.task.id,
      target: target.id,
      type: "smoothstep",
      animated: target.state === "running" || target.state === "leased",
      markerEnd: { type: MarkerType.ArrowClosed },
      className: `workflow-edge ${target.state}`
    }))
  );

  const maxLayer = Math.max(0, ...workflow.map((node) => node.layer));
  return {
    workflow,
    nodes,
    edges,
    height: Math.max(360, (maxLayer + 1) * 150 + 80)
  };
}

function WorkflowTaskLabel({ node }: { node: WorkflowNode }) {
  const upstream = node.upstream.map((task) => task.name).join(", ") || "root";
  const downstream = node.downstream.map((task) => task.name).join(", ") || "terminal";
  return (
    <div className="workflow-node-label">
      <div><strong>{node.task.name}</strong><StateBadge state={node.task.state} /></div>
      <p><span>Upstream</span>{upstream}</p>
      <p><span>Downstream</span>{downstream}</p>
      {node.missingDependencies.length ? <small>missing {node.missingDependencies.length}</small> : null}
    </div>
  );
}

function SummaryPill({ label, value }: { label: string; value: number }) {
  return (
    <div className="summary-pill">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
