import type { CollectorRow } from "@/api";

export function isCollectorRunning(collector: CollectorRow): boolean {
  return Boolean(collector.is_running);
}

export function isCollectorDisabled(collector: CollectorRow): boolean {
  return collector.status === "disabled";
}

export function isCollectorSupportedOnHost(collector: CollectorRow): boolean {
  return collector.supported_on_host !== false;
}

export function listSupportedCollectors(collectors: CollectorRow[]): CollectorRow[] {
  return collectors.filter((collector) => !isCollectorDisabled(collector) && isCollectorSupportedOnHost(collector));
}

export function listUnsupportedCollectors(collectors: CollectorRow[]): CollectorRow[] {
  return collectors.filter((collector) => !isCollectorDisabled(collector) && !isCollectorSupportedOnHost(collector));
}

export function listIdleCollectors(collectors: CollectorRow[]): CollectorRow[] {
  return collectors.filter(
    (collector) => !isCollectorDisabled(collector) && isCollectorSupportedOnHost(collector) && !isCollectorRunning(collector),
  );
}

export function listRunningCollectors(collectors: CollectorRow[]): CollectorRow[] {
  return collectors.filter((collector) => isCollectorSupportedOnHost(collector) && isCollectorRunning(collector));
}

export function summarizeCollectorFleet(collectors: CollectorRow[]) {
  const configured = collectors.filter((collector) => !isCollectorDisabled(collector));
  const supported = listSupportedCollectors(collectors);
  const unsupported = listUnsupportedCollectors(collectors);
  const running = listRunningCollectors(supported);
  const idle = listIdleCollectors(supported);
  return {
    total: collectors.length,
    configured: configured.length,
    supported: supported.length,
    unsupported: unsupported.length,
    running: running.length,
    idle: idle.length,
    idleCollectors: idle,
    unsupportedCollectors: unsupported,
  };
}
