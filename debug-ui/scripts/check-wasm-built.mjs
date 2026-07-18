#!/usr/bin/env node
// Runs automatically before `npm run dev` / `npm run build` (npm's
// pre<script> lifecycle hook). worker.ts imports "../wasm/qr_lab_wasm.js",
// which only exists after `npm run build:wasm` has run — without this
// check, a missing wasm package surfaces as an opaque Vite/tsc "cannot
// resolve module" error instead of telling the developer what to do.
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const debugUiDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const marker = path.join(debugUiDir, "src", "wasm", "qr_lab_wasm.js");

if (!existsSync(marker)) {
  console.error(
    [
      "",
      "error: debug-ui/src/wasm/qr_lab_wasm.js is missing.",
      "",
      "The scanner worker imports the wasm-pack build output, which isn't",
      "checked into git. Build it first:",
      "",
      "  npm run build:wasm",
      "",
    ].join("\n"),
  );
  process.exit(1);
}
