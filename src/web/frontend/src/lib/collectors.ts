import type { CollectorRow } from "@/api";

export function isCollectorRunning(collector: CollectorRow): boolean {
  return Boolean(collector.is_running);
}

export function isCollectorDisabled(collector: CollectorRow): boolean {
  return collector.status === "disabled";
}

export function listIdleCollectors(collectors: CollectorRow[]): CollectorRow[] {
  return collectors.filter((collector) => !isCollectorDisabled(collector) && !isCollectorRunning(collector));
}

export function listRunningCollectors(collectors: CollectorRow[]): CollectorRow[] {
  return collectors.filter((collector) => isCollectorRunning(collector));
}

export function summarizeCollectorFleet(collectors: CollectorRow[]) {
  const enabled = collectors.filter((collector) => !isCollectorDisabled(collector));
  const running = listRunningCollectors(enabled);
  const idle = listIdleCollectors(enabled);
  return {
    total: collectors.length,
    enabled: enabled.length,
    running: running.length,
    idle: idle.length,
    idleCollectors: idle,
  };
}
