import type { HealthResponse, OverviewResponse, ServiceRow, WorkspaceMapResponse, WorkspaceMapping, WorkspaceRuntimeApp } from "@/api";

export type AttentionTone = "destructive" | "warning" | "info";

export type AttentionItem = {
  id: string;
  tone: AttentionTone;
  title: string;
  detail: string;
  href?: string;
};

export type InventoryApplication = {
  app: WorkspaceRuntimeApp;
  service?: ServiceRow;
  mappedServiceId?: string | null;
};

export type InventoryDataStore = {
  id: string;
  label: string;
  service?: ServiceRow;
  monitored: boolean;
  processDetected?: boolean;
  hint?: string;
};

export type SystemsInventory = {
  attention: AttentionItem[];
  hostname: string;
  hostService?: ServiceRow;
  topProcesses: NonNullable<OverviewResponse["runtime"]["processes"]>;
  containers: NonNullable<OverviewResponse["runtime"]["containers"]>;
  applications: InventoryApplication[];
  dataStores: InventoryDataStore[];
  platformServices: ServiceRow[];
  otherServices: ServiceRow[];
  unmappedServices: string[];
  collectorsRunning: number;
  collectorsExpected: number;
  idleCollectorNames: string[];
};

const DATABASE_HINTS = [
  "postgres",
  "postgresql",
  "mssql",
  "sqlserver",
  "sql server",
  "mysql",
  "mariadb",
  "mongodb",
  "mongo",
  "redis",
  "supabase",
  "sqlite",
  "cockroach",
  "cassandra",
  "elasticsearch",
  "mssqlserver",
  "sqlservr",
];

const DATABASE_PROCESS_NAMES = [
  "postgres",
  "postgres.exe",
  "sqlservr",
  "sqlservr.exe",
  "mysqld",
  "mysqld.exe",
  "mongod",
  "mongod.exe",
  "redis-server",
  "redis-server.exe",
];

export function isDatabaseIdentity(value: string): boolean {
  const haystack = value.toLowerCase();
  return DATABASE_HINTS.some((hint) => haystack.includes(hint));
}

export function isHostServiceId(serviceId: string, hostname?: string | null): boolean {
  const id = serviceId.toLowerCase();
  if (id === "host" || id === "localhost") return true;
  if (hostname && id === hostname.toLowerCase()) return true;
  return false;
}

export function serviceNeedsAttention(status: string | null | undefined): boolean {
  return ["critical", "degraded", "elevated"].includes(String(status ?? "").toLowerCase());
}

export function formatHostCpuPercent(cpuPercent: number | null | undefined): string {
  if (cpuPercent == null) return "—";
  return Number.isInteger(cpuPercent) ? `${cpuPercent}%` : `${cpuPercent.toFixed(1)}%`;
}

export function resolveMappedServiceId(
  app: WorkspaceRuntimeApp,
  mappings: WorkspaceMapping[],
): string | null {
  const byName = mappings.find(
    (mapping) => mapping.service_id.toLowerCase() === app.name.toLowerCase(),
  );
  if (byName) return byName.service_id;
  if (app.project_path) {
    const byProject = mappings.find((mapping) => mapping.project_path === app.project_path);
    if (byProject) return byProject.service_id;
  }
  return null;
}

function serviceById(services: ServiceRow[], serviceId: string): ServiceRow | undefined {
  return services.find((service) => service.service_id === serviceId);
}

function buildDataStores(
  services: ServiceRow[],
  processes: NonNullable<OverviewResponse["runtime"]["processes"]>,
): InventoryDataStore[] {
  const stores = new Map<string, InventoryDataStore>();

  for (const service of services) {
    if (!isDatabaseIdentity(service.service_id)) continue;
    stores.set(service.service_id, {
      id: service.service_id,
      label: service.service_id,
      service,
      monitored: true,
      hint: "Events attributed to this data store",
    });
  }

  for (const process of processes) {
    const name = process.name.toLowerCase();
    if (!DATABASE_PROCESS_NAMES.some((hint) => name.includes(hint.replace(".exe", "")))) continue;
    const id = process.name.replace(/\.exe$/i, "");
    if (stores.has(id)) {
      const existing = stores.get(id)!;
      existing.processDetected = true;
      continue;
    }
    stores.set(id, {
      id,
      label: process.name,
      monitored: false,
      processDetected: true,
      hint: "Process detected — tail DB logs or ingest app metrics for query/load signals",
    });
  }

  return [...stores.values()].sort((left, right) => {
    const leftRisk = serviceNeedsAttention(left.service?.status) ? 0 : 1;
    const rightRisk = serviceNeedsAttention(right.service?.status) ? 0 : 1;
    if (leftRisk !== rightRisk) return leftRisk - rightRisk;
    return left.label.localeCompare(right.label);
  });
}

function buildApplications(
  apps: WorkspaceRuntimeApp[],
  services: ServiceRow[],
  mappings: WorkspaceMapping[],
): InventoryApplication[] {
  return apps
    .filter((app) => !isDatabaseIdentity(app.name) && !isDatabaseIdentity(app.runtime))
    .map((app) => {
      const mappedServiceId = resolveMappedServiceId(app, mappings);
      const service = mappedServiceId ? serviceById(services, mappedServiceId) : undefined;
      return { app, service, mappedServiceId };
    })
    .sort((left, right) => {
      const leftRisk = serviceNeedsAttention(left.service?.status) ? 0 : 1;
      const rightRisk = serviceNeedsAttention(right.service?.status) ? 0 : 1;
      if (leftRisk !== rightRisk) return leftRisk - rightRisk;
      return (left.app.display_name ?? left.app.name).localeCompare(right.app.display_name ?? right.app.name);
    });
}

function buildAttentionItems(
  inventory: Omit<SystemsInventory, "attention">,
  inferraHealth?: HealthResponse | null,
): AttentionItem[] {
  const items: AttentionItem[] = [];

  if (
    inventory.collectorsExpected > 0 &&
    inventory.collectorsRunning === 0
  ) {
    items.push({
      id: "collectors-stopped",
      tone: "warning",
      title: "Collectors are not running",
      detail:
        "Log and metric collectors are configured but stopped — only workspace process scans and stale DB events are visible. Start collectors from Control.",
      href: "/control",
    });
  } else if (
    inventory.collectorsExpected > 0 &&
    inventory.collectorsRunning < inventory.collectorsExpected
  ) {
    items.push({
      id: "collectors-partial",
      tone: "info",
      title: "Some collectors are idle",
      detail: `${inventory.collectorsRunning} of ${inventory.collectorsExpected} collectors running${
        inventory.idleCollectorNames.length ? ` (${inventory.idleCollectorNames.join(", ")})` : ""
      }. Check Control for why.`,
      href: "/control",
    });
  }

  if (inferraHealth?.status === "degraded" || inferraHealth?.storage_writes_ok === false) {
    items.push({
      id: "inferra-runtime-degraded",
      tone: "destructive",
      title: "Inferra API runtime degraded",
      detail:
        inferraHealth.degraded_reasons?.join(" · ") ??
        "Storage probes failed — collectors and incidents may be impaired.",
      href: "/control",
    });
  }

  const pushServiceAttention = (service: ServiceRow, prefix: string) => {
    if (!serviceNeedsAttention(service.status)) return;
    items.push({
      id: `service-${service.service_id}`,
      tone: service.status === "critical" ? "destructive" : "warning",
      title: `${prefix}${service.service_id}`,
      detail: `${service.error_count ?? 0} errors · ${service.event_count ?? 0} events · status ${service.status}`,
      href: `/systems/${encodeURIComponent(service.service_id)}`,
    });
  };

  if (inventory.hostService) pushServiceAttention(inventory.hostService, "Server ");
  for (const { service, app } of inventory.applications) {
    if (service) pushServiceAttention(service, `${app.display_name ?? app.name}: `);
  }
  for (const store of inventory.dataStores) {
    if (store.service) pushServiceAttention(store.service, `${store.label}: `);
    else if (store.processDetected) {
      items.push({
        id: `db-unmonitored-${store.id}`,
        tone: "info",
        title: `${store.label} running without log signals`,
        detail: store.hint ?? "Wire log ingest or app metrics to monitor load and queries.",
      });
    }
  }
  for (const service of [...inventory.platformServices, ...inventory.otherServices]) {
    pushServiceAttention(service, "");
  }

  for (const serviceId of inventory.unmappedServices.filter((id) => !isHostServiceId(id)).slice(0, 6)) {
    items.push({
      id: `unmapped-${serviceId}`,
      tone: "info",
      title: `Unmapped service "${serviceId}"`,
      detail: "Inferra sees events but has no workspace project owner yet.",
      href: `/systems/${encodeURIComponent(serviceId)}`,
    });
  }

  for (const container of inventory.containers) {
    const state = container.state.toLowerCase();
    if (state === "running") continue;
    items.push({
      id: `container-${container.name}`,
      tone: "warning",
      title: `Container ${container.name}`,
      detail: `State ${container.state} · image ${container.image}`,
    });
  }

  return items;
}

export function buildSystemsInventory(
  services: ServiceRow[],
  overview: OverviewResponse | null | undefined,
  workspace: WorkspaceMapResponse | null | undefined,
  inferraHealth?: HealthResponse | null,
  collectors?: { running: number; supported: number; idleNames?: string[] },
): SystemsInventory {
  const hostname = overview?.runtime?.hostname ?? "This host";
  const processes = overview?.runtime?.processes ?? [];
  const containers = overview?.runtime?.containers ?? [];
  const mappings = workspace?.service_mappings ?? [];
  const runtimeApps = workspace?.runtime_apps ?? [];
  const unmappedServices = workspace?.unmapped_services ?? [];

  const mappedServiceIds = new Set(mappings.map((mapping) => mapping.service_id));
  const appMappedIds = new Set(
    runtimeApps.map((app) => resolveMappedServiceId(app, mappings)).filter((id): id is string => Boolean(id)),
  );

  let hostService: ServiceRow | undefined;
  const platformServices: ServiceRow[] = [];
  const otherServices: ServiceRow[] = [];

  for (const service of services) {
    if (isHostServiceId(service.service_id, hostname)) {
      hostService = service;
      continue;
    }
    if (isDatabaseIdentity(service.service_id)) continue;
    if (appMappedIds.has(service.service_id)) continue;
    if (mappedServiceIds.has(service.service_id) && !appMappedIds.has(service.service_id)) {
      platformServices.push(service);
      continue;
    }
    otherServices.push(service);
  }

  const applications = buildApplications(runtimeApps, services, mappings);
  const dataStores = buildDataStores(services, processes);

  const base = {
    hostname,
    hostService,
    topProcesses: [...processes].sort((left, right) => right.cpu_percent - left.cpu_percent).slice(0, 6),
    containers,
    applications,
    dataStores,
    platformServices: platformServices.sort((left, right) => left.service_id.localeCompare(right.service_id)),
    otherServices: otherServices.sort((left, right) => left.service_id.localeCompare(right.service_id)),
    unmappedServices,
    collectorsRunning: collectors?.running ?? 0,
    collectorsExpected: collectors?.supported ?? 0,
    idleCollectorNames: collectors?.idleNames ?? [],
  };

  return {
    ...base,
    attention: buildAttentionItems(base, inferraHealth),
  };
}

export function hasInventoryContent(inventory: SystemsInventory): boolean {
  return Boolean(
    inventory.hostService ||
      inventory.applications.length ||
      inventory.dataStores.length ||
      inventory.containers.length ||
      inventory.platformServices.length ||
      inventory.otherServices.length,
  );
}
