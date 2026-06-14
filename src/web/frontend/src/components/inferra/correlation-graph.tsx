import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BaseEdge,
  Controls,
  EdgeLabelRenderer,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  ReactFlowProvider,
  getSmoothStepPath,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Edge,
  type EdgeProps,
  type Node,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";

import type { IncidentRow, ServiceRow, TopologyEdge, WorkspaceMapping, WorkspaceRuntimeApp } from "@/api";
import { ServiceHealthBadge, SeverityIndicator } from "@/components/inferra/health";
import { loadGraphLayout, mergeNodePositions, saveGraphLayout, clearGraphLayout } from "@/lib/graph-layout-storage";
import { cn } from "@/lib/utils";
import { CircleDot, RotateCcw, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export type GraphNodeKind = "service" | "incident" | "app";

export type GraphSelection = {
  kind: GraphNodeKind;
  id: string;
  label: string;
} | null;

type GraphNodeData = {
  label: string;
  kind: GraphNodeKind;
  status?: string;
  severity?: number;
  event_count?: number;
  error_ratio?: number;
  subtitle?: string;
  selected?: boolean;
  connected?: boolean;
};

type GraphEdgeKind = "topology" | "incident" | "correlation" | "mapping";

type PipeEdgeData = {
  kind: GraphEdgeKind;
  active?: boolean;
  label?: string;
};

const EDGE_PIPE_COLORS: Record<GraphEdgeKind, { track: string; flow: string }> = {
  topology: { track: "color-mix(in srgb, var(--accent) 22%, var(--border))", flow: "var(--accent)" },
  incident: { track: "color-mix(in srgb, var(--warning) 24%, var(--border))", flow: "var(--warning)" },
  correlation: { track: "color-mix(in srgb, var(--graph-link-correlation) 22%, var(--border))", flow: "var(--graph-link-correlation)" },
  mapping: { track: "color-mix(in srgb, var(--success) 22%, var(--border))", flow: "var(--success)" },
};

type CorrelationGraphProps = {
  services: ServiceRow[];
  incidents: IncidentRow[];
  edges: TopologyEdge[];
  runtimeApps?: WorkspaceRuntimeApp[];
  serviceMappings?: WorkspaceMapping[];
  unmappedServices?: string[];
  selection?: GraphSelection;
  onSelect?: (selection: GraphSelection) => void;
  className?: string;
};

const edgeTypes = { pipe: PipeEdge };

function PipeEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  markerEnd,
}: EdgeProps) {
  const edgeData = (data ?? {}) as PipeEdgeData;
  const kind = edgeData.kind ?? "topology";
  const active = edgeData.active ?? false;
  const colors = EDGE_PIPE_COLORS[kind];
  const [edgePath, labelX, labelY] = getSmoothStepPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
    borderRadius: 20,
  });

  return (
    <>
      <BaseEdge
        id={id}
        path={edgePath}
        markerEnd={markerEnd}
        style={{
          stroke: colors.track,
          strokeWidth: active ? 11 : 9,
          strokeLinecap: "round",
        }}
      />
      <path
        d={edgePath}
        fill="none"
        stroke={colors.flow}
        strokeWidth={active ? 3.5 : 2.75}
        strokeLinecap="round"
        className={cn("graph-pipe-flow", `graph-pipe-flow-${kind}`, active && "graph-pipe-flow-active")}
      />
      {edgeData.label ? (
        <EdgeLabelRenderer>
          <div
            className="graph-edge-label pointer-events-none rounded-sm border border-border bg-card/95 px-1.5 py-0.5 font-data text-[10px] text-muted-foreground shadow-sm"
            style={{
              position: "absolute",
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
            }}
          >
            {edgeData.label}
          </div>
        </EdgeLabelRenderer>
      ) : null}
    </>
  );
}

export function CorrelationGraph(props: CorrelationGraphProps) {
  return (
    <ReactFlowProvider>
      <CorrelationGraphCanvas {...props} />
    </ReactFlowProvider>
  );
}

function CorrelationGraphCanvas({
  services,
  incidents,
  edges,
  runtimeApps = [],
  serviceMappings = [],
  unmappedServices = [],
  selection,
  onSelect,
  className,
}: CorrelationGraphProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [hideUnmapped, setHideUnmapped] = useState(true);
  const graph = useMemo(
    () =>
      buildFlowGraph(
        services,
        incidents,
        edges,
        runtimeApps,
        serviceMappings,
        selection?.id ?? null,
        {
          searchQuery,
          hideUnmapped,
          unmappedServices,
        },
      ),
    [edges, hideUnmapped, incidents, runtimeApps, searchQuery, selection?.id, serviceMappings, services, unmappedServices],
  );
  const savedLayoutRef = useRef(loadGraphLayout());
  const [nodes, setNodes, onNodesChange] = useNodesState(graph.nodes);
  const [flowEdges, setEdges, onEdgesChange] = useEdgesState(graph.edges);
  const { fitView, getNodes, getViewport, setViewport } = useReactFlow();

  useEffect(() => {
    setNodes(mergeNodePositions(graph.nodes, savedLayoutRef.current));
    setEdges(graph.edges);
  }, [graph.edges, graph.nodes, setEdges, setNodes]);

  const persistLayout = useCallback(() => {
    const currentNodes = getNodes();
    const positions = Object.fromEntries(
      currentNodes.map((node) => [node.id, { x: node.position.x, y: node.position.y }]),
    );
    const layout = {
      version: 1 as const,
      nodes: positions,
      viewport: getViewport(),
    };
    savedLayoutRef.current = layout;
    saveGraphLayout(layout);
  }, [getNodes, getViewport]);

  const handleNodeDragStop = useCallback(() => {
    persistLayout();
  }, [persistLayout]);

  const handleMoveEnd = useCallback(() => {
    persistLayout();
  }, [persistLayout]);

  const resetLayout = useCallback(() => {
    clearGraphLayout();
    savedLayoutRef.current = null;
    setNodes(graph.nodes);
    setEdges(graph.edges);
    void fitView({ padding: 0.12, duration: 350, maxZoom: 1.1 });
  }, [fitView, graph.edges, graph.nodes, setEdges, setNodes]);

  useEffect(() => {
    if (!graph.nodes.length) return;
    const savedViewport = savedLayoutRef.current?.viewport;
    if (savedViewport) {
      void setViewport(savedViewport, { duration: 0 });
      return;
    }
    const timer = window.setTimeout(() => {
      void fitView({ padding: 0.12, duration: 450, maxZoom: 1.1 });
    }, 60);
    return () => window.clearTimeout(timer);
  }, [fitView, graph.nodes.length, setViewport]);

  const handleNodeClick = useCallback(
    (_: React.MouseEvent, node: Node<GraphNodeData>) => {
      onSelect?.({
        kind: node.data.kind,
        id: node.id,
        label: node.data.label,
      });
    },
    [onSelect],
  );

  const handlePaneClick = useCallback(() => {
    onSelect?.(null);
  }, [onSelect]);

  const stats = graph.stats;

  return (
    <div className={cn("flex min-h-0 flex-col", className)}>
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-border px-4 py-2.5">
        <div className="flex min-w-[220px] flex-1 items-center gap-2">
          <Search className="size-4 text-muted-foreground" />
          <Input
            aria-label="Search graph nodes"
            className="h-8 max-w-xs"
            placeholder="Filter nodes…"
            value={searchQuery}
            onChange={(event) => setSearchQuery(event.target.value)}
          />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            size="sm"
            variant={hideUnmapped ? "default" : "outline"}
            onClick={() => setHideUnmapped((value) => !value)}
          >
            {hideUnmapped ? "Hiding unmapped" : "Show unmapped"}
          </Button>
          <Button type="button" size="sm" variant="outline" onClick={resetLayout}>
            <RotateCcw className="size-4" />
            Reset layout
          </Button>
        </div>
        <div className="flex flex-wrap gap-3 font-data text-xs text-muted-foreground">
          <span>{stats.services} services</span>
          <span>{stats.apps} apps</span>
          <span>{stats.incidents} incidents</span>
          <span>{stats.links} pipes</span>
          {hideUnmapped && unmappedServices.length ? <span>{unmappedServices.length} unmapped hidden</span> : null}
        </div>
        <GraphLegend />
      </div>
      <div className="graph-canvas-shell relative min-h-0 flex-1 overflow-hidden bg-panel-inset">
        <ReactFlow
          nodes={nodes}
          edges={flowEdges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onNodeDragStop={handleNodeDragStop}
          onMoveEnd={handleMoveEnd}
          nodeTypes={{ inferra: InferraNode }}
          edgeTypes={edgeTypes}
          onNodeClick={handleNodeClick}
          onPaneClick={handlePaneClick}
          nodesDraggable
          fitView
          minZoom={0.2}
          maxZoom={1.8}
          proOptions={{ hideAttribution: true }}
          defaultEdgeOptions={{ type: "pipe" }}
        >
          <Background color="var(--border)" gap={22} size={1} />
          <MiniMap
            pannable
            zoomable
            nodeStrokeWidth={2}
            className="!rounded-sm !border-border !bg-card/90"
            maskColor="color-mix(in srgb, var(--background) 72%, transparent)"
          />
          <Controls showInteractive={false} className="!rounded-sm !border-border !bg-card !shadow-none" />
        </ReactFlow>
      </div>
    </div>
  );
}

function GraphLegend() {
  const items: Array<{ label: string; color: string }> = [
    { label: "Topology", color: "var(--accent)" },
    { label: "Incident", color: "var(--warning)" },
    { label: "Correlation", color: "var(--graph-link-correlation)" },
    { label: "Workspace app", color: "var(--success)" },
  ];
  return (
    <div className="flex flex-wrap items-center gap-3 font-data text-[10px] text-muted-foreground">
      {items.map((item) => (
        <span key={item.label} className="inline-flex items-center gap-1.5">
          <span className="graph-legend-pipe" style={{ background: item.color }} />
          {item.label}
        </span>
      ))}
    </div>
  );
}

function InferraNode({ data, selected }: { data: GraphNodeData; selected?: boolean }) {
  const isSelected = selected || data.selected;
  const isCriticalIncident = data.kind === "incident" && (data.severity ?? 0) >= 3;
  const errorPct = Math.round((data.error_ratio ?? 0) * 100);

  return (
    <div
      className={cn(
        "graph-node w-[196px] rounded-md border bg-card p-3 transition-all duration-200",
        isSelected ? "border-accent graph-node-selected" : data.connected ? "border-accent/50 graph-node-connected" : "border-border",
        isCriticalIncident ? "graph-node-critical" : null,
        data.kind === "app" ? "graph-node-app" : null,
      )}
    >
      <Handle type="target" position={Position.Left} className="!h-2 !w-2 !border-border !bg-card" />
      <Handle type="source" position={Position.Right} className="!h-2 !w-2 !border-border !bg-card" />
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate font-data text-xs font-semibold">{data.label}</p>
          <p className="mt-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">{data.kind}</p>
          {data.subtitle ? <p className="mt-1 truncate text-[10px] text-muted-foreground">{data.subtitle}</p> : null}
        </div>
        {data.kind === "incident" ? (
          <SeverityIndicator value={data.severity} />
        ) : data.kind === "service" ? (
          <ServiceHealthBadge status={data.status} />
        ) : (
          <CircleDot className="size-4 shrink-0 text-accent" />
        )}
      </div>
      {data.kind === "service" ? (
        <div className="mt-2 flex items-center justify-between gap-2 font-data text-[10px] text-muted-foreground">
          <span>{data.event_count ?? 0} events</span>
          <span className={errorPct > 0 ? "text-[var(--warning)]" : undefined}>{errorPct}% err</span>
        </div>
      ) : null}
    </div>
  );
}

function buildFlowGraph(
  services: ServiceRow[],
  incidents: IncidentRow[],
  topology: TopologyEdge[],
  runtimeApps: WorkspaceRuntimeApp[],
  serviceMappings: WorkspaceMapping[],
  selectedId?: string | null,
  options?: {
    searchQuery?: string;
    hideUnmapped?: boolean;
    unmappedServices?: string[];
  },
) {
  const searchQuery = options?.searchQuery?.trim().toLowerCase() ?? "";
  const hideUnmapped = options?.hideUnmapped ?? false;
  const unmappedSet = new Set((options?.unmappedServices ?? []).map((service) => service.toLowerCase()));

  const filteredServices = services
    .filter((service) => {
      if (hideUnmapped && unmappedSet.has(service.service_id.toLowerCase())) return false;
      if (!searchQuery) return true;
      return service.service_id.toLowerCase().includes(searchQuery);
    })
    .slice(0, 32);

  const serviceNodes: Array<{ id: string; kind: "service"; anchor?: string; data: GraphNodeData }> = filteredServices
    .map((service) => ({
      id: service.service_id,
      kind: "service" as const,
      data: {
        label: service.service_id,
        kind: "service",
        status: service.status,
        event_count: service.event_count,
        error_ratio: service.error_ratio,
        subtitle: service.last_event_at ? "recent activity" : "idle",
      },
    }));

  const serviceIds = new Set(serviceNodes.map((node) => node.id));

  const mappedApps = runtimeApps
    .filter((app) => {
      const label = (app.display_name || app.name).toLowerCase();
      if (searchQuery && !label.includes(searchQuery) && !app.name.toLowerCase().includes(searchQuery)) {
        return false;
      }
      const mappedService = resolveMappedService(app, serviceMappings);
      return !mappedService || serviceIds.has(mappedService);
    })
    .slice(0, 16);

  const appNodes = mappedApps.map((app) => {
    const mappedService = resolveMappedService(app, serviceMappings);
    return {
      id: `app:${app.name}`,
      kind: "app" as const,
      anchor: mappedService ?? undefined,
      data: {
        label: app.display_name || app.name,
        kind: "app" as const,
        subtitle: app.runtime,
      },
    };
  });

  const incidentNodes = incidents
    .filter((incident) => {
      if (!searchQuery) return true;
      const haystack = `${incident.incident_id} ${incident.primary_service ?? ""}`.toLowerCase();
      return haystack.includes(searchQuery);
    })
    .slice(0, 14)
    .map((incident) => ({
      id: incident.incident_id,
      kind: "incident" as const,
      anchor: incident.primary_service || undefined,
      data: {
        label: incident.primary_service || incident.incident_id,
        kind: "incident" as const,
        severity: incident.severity,
        subtitle: `${incident.event_count ?? 0} events`,
      },
    }));

  const layoutNodes = [
    ...serviceNodes.map((node) => ({ id: node.id, kind: node.kind, anchor: node.anchor })),
    ...appNodes.map((node) => ({ id: node.id, kind: node.kind, anchor: node.anchor })),
    ...incidentNodes.map((node) => ({ id: node.id, kind: node.kind, anchor: node.anchor })),
  ];

  const flowEdges: Array<{ id: string; source: string; target: string; kind: GraphEdgeKind; label?: string }> = [];

  for (const edge of topology) {
    if (!serviceIds.has(edge.source) || !serviceIds.has(edge.target)) continue;
    flowEdges.push({
      id: `topology:${edge.source}:${edge.target}`,
      source: edge.source,
      target: edge.target,
      kind: "topology",
      label: edge.relation_type ?? edge.type,
    });
  }

  for (const incident of incidents) {
    const servicesForIncident = [incident.primary_service, ...(incident.affected_services ?? [])].filter(
      (service): service is string => Boolean(service && serviceIds.has(service)),
    );
    for (const service of servicesForIncident) {
      flowEdges.push({
        id: `incident:${incident.incident_id}:${service}`,
        source: service,
        target: incident.incident_id,
        kind: "incident",
      });
    }
    for (let i = 0; i < servicesForIncident.length; i += 1) {
      for (let j = i + 1; j < servicesForIncident.length; j += 1) {
        const left = servicesForIncident[i];
        const right = servicesForIncident[j];
        flowEdges.push({
          id: `correlation:${incident.incident_id}:${left}:${right}`,
          source: left,
          target: right,
          kind: "correlation",
        });
      }
    }
  }

  for (const app of mappedApps) {
    const mappedService = resolveMappedService(app, serviceMappings);
    if (!mappedService || !serviceIds.has(mappedService)) continue;
    flowEdges.push({
      id: `mapping:${app.name}:${mappedService}`,
      source: `app:${app.name}`,
      target: mappedService,
      kind: "mapping",
    });
  }

  if (serviceNodes.length === 1 && mappedApps.length > 0) {
    const loneService = serviceNodes[0].id;
    for (const app of mappedApps) {
      const mappedService = resolveMappedService(app, serviceMappings);
      if (mappedService) continue;
      flowEdges.push({
        id: `mapping:${app.name}:${loneService}`,
        source: `app:${app.name}`,
        target: loneService,
        kind: "mapping",
      });
    }
  }

  for (const service of services) {
    for (const incident of service.active_incidents ?? []) {
      for (const affected of incident.affected_services ?? []) {
        if (!affected || affected === service.service_id || !serviceIds.has(affected)) continue;
        flowEdges.push({
          id: `active:${service.service_id}:${affected}:${incident.incident_id}`,
          source: service.service_id,
          target: affected,
          kind: "correlation",
        });
      }
    }
  }

  const traceOwners = new Map<string, string[]>();
  for (const service of services) {
    const traceId = service.latest_trace_summary?.trace_id;
    if (!traceId || !serviceIds.has(service.service_id)) continue;
    const owners = traceOwners.get(traceId) ?? [];
    owners.push(service.service_id);
    traceOwners.set(traceId, owners);
  }
  for (const owners of traceOwners.values()) {
    const uniqueOwners = [...new Set(owners)];
    for (let i = 0; i < uniqueOwners.length; i += 1) {
      for (let j = i + 1; j < uniqueOwners.length; j += 1) {
        flowEdges.push({
          id: `trace:${uniqueOwners[i]}:${uniqueOwners[j]}`,
          source: uniqueOwners[i],
          target: uniqueOwners[j],
          kind: "correlation",
        });
      }
    }
  }

  const connectedIds = selectedId ? collectConnectedIds(selectedId, flowEdges) : new Set<string>();
  const positions = computeLayout(layoutNodes);

  const nodes: Node<GraphNodeData>[] = [...serviceNodes, ...appNodes, ...incidentNodes].map((node) => {
    const position = positions.get(node.id) ?? { x: 0, y: 0 };
    return {
      id: node.id,
      type: "inferra",
      position,
      sourcePosition: Position.Right,
      targetPosition: Position.Left,
      data: {
        ...node.data,
        selected: node.id === selectedId,
        connected: connectedIds.has(node.id) && node.id !== selectedId,
      },
      selected: node.id === selectedId,
    };
  });

  const edges: Edge[] = dedupeEdges(flowEdges).map((edge) => {
    const active =
      Boolean(selectedId) &&
      (edge.source === selectedId ||
        edge.target === selectedId ||
        (connectedIds.has(edge.source) && connectedIds.has(edge.target)));
    return {
      id: edge.id,
      type: "pipe",
      source: edge.source,
      target: edge.target,
      data: {
        kind: edge.kind,
        active,
        label: edge.kind === "topology" ? edge.label : undefined,
      } satisfies PipeEdgeData,
      className: `graph-edge graph-edge-${edge.kind}`,
    };
  });

  return {
    nodes,
    edges,
    stats: {
      services: serviceNodes.length,
      apps: appNodes.length,
      incidents: incidentNodes.length,
      links: edges.length,
    },
  };
}

function resolveMappedService(app: WorkspaceRuntimeApp, mappings: WorkspaceMapping[]): string | null {
  const projectPath = app.project_path ?? app.app_location?.project_path ?? null;
  if (!projectPath) return null;
  const normalized = projectPath.replace(/\\/g, "/").toLowerCase();
  const match = mappings.find((mapping) => mapping.project_path.replace(/\\/g, "/").toLowerCase() === normalized);
  return match?.service_id ?? null;
}

function collectConnectedIds(selectedId: string, edges: Array<{ source: string; target: string }>) {
  const connected = new Set<string>([selectedId]);
  for (const edge of edges) {
    if (edge.source === selectedId) connected.add(edge.target);
    if (edge.target === selectedId) connected.add(edge.source);
  }
  return connected;
}

function dedupeEdges<T extends { id: string }>(edges: T[]) {
  const seen = new Set<string>();
  return edges.filter((edge) => {
    if (seen.has(edge.id)) return false;
    seen.add(edge.id);
    return true;
  });
}

function computeLayout(nodes: Array<{ id: string; kind: GraphNodeKind; anchor?: string }>) {
  const positions = new Map<string, { x: number; y: number }>();
  const rowHeight = 108;
  const top = 32;
  const colApp = 24;
  const colService = 268;
  const colIncident = 548;

  const services = nodes
    .filter((node) => node.kind === "service")
    .sort((left, right) => left.id.localeCompare(right.id, undefined, { sensitivity: "base" }));
  const apps = nodes
    .filter((node) => node.kind === "app")
    .sort((left, right) => left.id.localeCompare(right.id, undefined, { sensitivity: "base" }));
  const incidents = nodes
    .filter((node) => node.kind === "incident")
    .sort((left, right) => left.id.localeCompare(right.id, undefined, { sensitivity: "base" }));

  const serviceRow = new Map<string, number>();
  services.forEach((service, index) => {
    serviceRow.set(service.id, index);
    positions.set(service.id, { x: colService, y: top + index * rowHeight });
  });

  const appsByService = new Map<string, Array<{ id: string; kind: GraphNodeKind; anchor?: string }>>();
  const unmappedApps: Array<{ id: string; kind: GraphNodeKind; anchor?: string }> = [];
  for (const app of apps) {
    if (app.anchor && serviceRow.has(app.anchor)) {
      const bucket = appsByService.get(app.anchor) ?? [];
      bucket.push(app);
      appsByService.set(app.anchor, bucket);
    } else {
      unmappedApps.push(app);
    }
  }

  let unmappedAppRow = 0;
  for (const app of unmappedApps) {
    positions.set(app.id, { x: colApp, y: top + unmappedAppRow * rowHeight });
    unmappedAppRow += 1;
  }

  for (const [serviceId, serviceApps] of appsByService) {
    const row = serviceRow.get(serviceId) ?? 0;
    serviceApps.forEach((app, index) => {
      positions.set(app.id, {
        x: colApp,
        y: top + row * rowHeight + index * 28,
      });
    });
  }

  let orphanIncidentRow = services.length;
  for (const incident of incidents) {
    if (incident.anchor && serviceRow.has(incident.anchor)) {
      const row = serviceRow.get(incident.anchor)!;
      positions.set(incident.id, { x: colIncident, y: top + row * rowHeight });
    } else {
      positions.set(incident.id, { x: colIncident, y: top + orphanIncidentRow * rowHeight });
      orphanIncidentRow += 1;
    }
  }

  return positions;
}
