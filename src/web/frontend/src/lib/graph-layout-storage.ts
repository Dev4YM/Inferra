const STORAGE_KEY = "inferra.graph.layout.v1";

export type SavedGraphLayout = {
  version: 1;
  nodes: Record<string, { x: number; y: number }>;
  viewport?: { x: number; y: number; zoom: number };
};

export function loadGraphLayout(): SavedGraphLayout | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<SavedGraphLayout>;
    if (!parsed?.nodes || typeof parsed.nodes !== "object") return null;
    return {
      version: 1,
      nodes: parsed.nodes,
      viewport: parsed.viewport,
    };
  } catch {
    return null;
  }
}

export function saveGraphLayout(layout: SavedGraphLayout): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(layout));
  } catch {
    // Ignore quota / private-mode failures.
  }
}

export function mergeNodePositions<T extends { id: string; position: { x: number; y: number } }>(
  nodes: T[],
  saved: SavedGraphLayout | null,
): T[] {
  if (!saved) return nodes;
  return nodes.map((node) => {
    const stored = saved.nodes[node.id];
    if (!stored) return node;
    return { ...node, position: { x: stored.x, y: stored.y } };
  });
}
