import { useMemo } from "react";
import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  type Edge,
  type Node,
  Position,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import type { IncidentRow, ServiceRow, TopologyEdge } from "@/api";
import { Badge } from "@/components/ui/badge";
import { ServiceHealthBadge, SeverityIndicator } from "@/components/inferra/health";

type GraphNodeData = {
  label: string;
  kind: "service" | "incident";
  status?: string;
  severity?: number;
};

export function CorrelationGraph({
  services,
  incidents,
  edges,
  activeService,
}: {
  services: ServiceRow[];
  incidents: IncidentRow[];
  edges: TopologyEdge[];
  activeService?: string | null;
}) {
  const graph = useMemo(() => buildFlowGraph(services, incidents, edges, activeService), [activeService, edges, incidents, services]);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap gap-2">
        <Badge variant="outline">{services.length} Services</Badge>
        <Badge variant="outline">{incidents.length} Incidents</Badge>
        <Badge variant="outline">{edges.length} Links</Badge>
      </div>
      <div className="h-[620px] overflow-hidden rounded-2xl border border-border/70 bg-background/35">
        <ReactFlow
          nodes={graph.nodes}
          edges={graph.edges}
          nodeTypes={{ inferra: InferraNode }}
          fitView
          minZoom={0.35}
          maxZoom={1.6}
          proOptions={{ hideAttribution: true }}
        >
          <Background color="var(--border)" gap={24} />
          <MiniMap pannable zoomable nodeStrokeWidth={3} />
          <Controls showInteractive={false} />
        </ReactFlow>
      </div>
    </div>
  );
}

function InferraNode({ data, selected }: { data: GraphNodeData; selected?: boolean }) {
  return (
    <div className={`w-56 rounded-2xl border bg-card/95 p-3 shadow-sm transition ${selected ? "border-primary/50 shadow-md" : "border-border/75"}`}>
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold">{data.label}</p>
          <p className="mt-1 text-xs text-muted-foreground">{data.kind === "incident" ? "Incident" : "Service"}</p>
        </div>
        {data.kind === "incident" ? <SeverityIndicator value={data.severity} /> : <ServiceHealthBadge status={data.status} />}
      </div>
    </div>
  );
}

function buildFlowGraph(services: ServiceRow[], incidents: IncidentRow[], topology: TopologyEdge[], activeService?: string | null) {
  const serviceNodes: Node<GraphNodeData>[] = services.slice(0, 24).map((service, index) => ({
    id: service.service_id,
    type: "inferra",
    position: { x: (index % 4) * 280, y: Math.floor(index / 4) * 140 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    data: { label: service.service_id, kind: "service", status: service.status },
  }));
  const serviceIds = new Set(serviceNodes.map((node) => node.id));
  const incidentNodes: Node<GraphNodeData>[] = incidents.slice(0, 10).map((incident, index) => ({
    id: incident.incident_id,
    type: "inferra",
    position: { x: 1120, y: index * 145 },
    sourcePosition: Position.Left,
    targetPosition: Position.Left,
    data: { label: incident.primary_service || incident.incident_id, kind: "incident", severity: incident.severity },
  }));
  const flowEdges: Edge[] = [];

  for (const edge of topology) {
    if (!serviceIds.has(edge.source) || !serviceIds.has(edge.target)) continue;
    const active = edge.source === activeService || edge.target === activeService;
    flowEdges.push({
      id: `topology:${edge.source}:${edge.target}`,
      source: edge.source,
      target: edge.target,
      animated: active,
      style: { stroke: active ? "var(--primary)" : "var(--border)", strokeWidth: active ? 2.5 : 1.5 },
    });
  }

  for (const incident of incidents) {
    for (const service of [incident.primary_service, ...(incident.affected_services ?? [])]) {
      if (!service || !serviceIds.has(service)) continue;
      flowEdges.push({
        id: `incident:${incident.incident_id}:${service}`,
        source: service,
        target: incident.incident_id,
        animated: incident.primary_service === activeService,
        style: { stroke: "var(--warning)", strokeWidth: 2 },
      });
    }
  }

  return { nodes: [...serviceNodes, ...incidentNodes], edges: flowEdges };
}
