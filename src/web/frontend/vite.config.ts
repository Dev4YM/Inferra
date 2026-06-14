import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import fs from "node:fs";
import path from "node:path";
import { defineConfig } from "vite";

function resolveInferraApiUrl(): string {
  if (process.env.INFERRA_API_URL) return process.env.INFERRA_API_URL;
  const root = path.resolve(__dirname, "../../..");
  const candidates = [
    path.join(root, "inferra.dev.toml"),
    path.join(root, "inferra.toml"),
  ];
  for (const filePath of candidates) {
    try {
      const text = fs.readFileSync(filePath, "utf8");
      const serverBlock = text.match(/\[server\][\s\S]*?(?=\n\[|$)/);
      const portMatch = serverBlock?.[0].match(/^port\s*=\s*(\d+)/m);
      const hostMatch = serverBlock?.[0].match(/^host\s*=\s*"([^"]+)"/m);
      const port = portMatch?.[1] ?? "7433";
      const host = hostMatch?.[1] ?? "127.0.0.1";
      return `http://${host}:${port}`;
    } catch {
      // try next config file
    }
  }
  return "http://127.0.0.1:7433";
}

const inferraApi = resolveInferraApiUrl();
const inferraWs = inferraApi.replace(/^http/, "ws");

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "../ui_dist",
    emptyOutDir: true,
    sourcemap: false,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("recharts") || id.includes("d3-")) return "charts";
          if (id.includes("@xyflow") || id.includes("zustand")) return "graph";
          if (id.includes("@tanstack")) return "query";
          return "vendor";
        },
      },
    },
  },
  server: {
    port: 5173,
    proxy: {
      "/api": inferraApi,
      "/healthz": inferraApi,
      "/readyz": inferraApi,
      "/ws": { target: inferraWs, ws: true },
    },
  },
});
