#!/usr/bin/env node
// Reuses the simulator's TypeScript lens implementation and the built WASM.
import { createServer } from "vite";
import { fileURLToPath } from "node:url";

const server = await createServer({
  root: fileURLToPath(new URL("..", import.meta.url)),
  configFile: false,
  server: { middlewareMode: true, hmr: false },
  appType: "custom",
});
try {
  const { run } = await server.ssrLoadModule("/scripts/fisheye-sweep.ts");
  await run(process.argv.slice(2));
} finally {
  await server.close();
}
