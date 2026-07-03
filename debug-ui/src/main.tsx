import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ScannerClient } from "./scanner/client";

// TODO(Task 6): remove this dev-only self-test once real image/video
// sources are wired up (see docs/superpowers/plans/2026-07-03-plan3-debug-
// ui-image-mode.md). It exists purely to manually sanity-check, end to
// end, that the worker loads the wasm-pack build and scan_rgba runs — a
// flat 64x64 gray frame has no finder patterns, so 0 finders is the
// expected (not broken) result.
async function runWasmSelfTest(): Promise<void> {
  const worker = new Worker(new URL("./scanner/worker.ts", import.meta.url), {
    type: "module",
  });
  const client = new ScannerClient(worker);
  await client.init();

  const size = 64;
  const rgba = new Uint8ClampedArray(size * size * 4);
  rgba.fill(128);
  for (let i = 3; i < rgba.length; i += 4) rgba[i] = 255; // opaque alpha

  const outcome = await client.scan(rgba, size, size, { maxDim: 0, withTrace: false });
  console.log("wasm self-test result", outcome);
}

function WasmSelfTestButton() {
  return (
    <button
      onClick={() => {
        runWasmSelfTest().catch((err: unknown) => {
          console.error("wasm self-test failed", err);
        });
      }}
    >
      wasm self-test
    </button>
  );
}

// Placeholder root — Plan 3's later tasks replace this with the real
// viewport/overlay/panel app (see docs/superpowers/plans for the shape).
createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <div>
      <p>qrk debug ui</p>
      <WasmSelfTestButton />
    </div>
  </StrictMode>,
);
