// Scanner Web Worker: loads the qrk-wasm package once, then services scan
// requests off the main thread so a 24MP frame's detection cost never
// blocks the UI. Kept thin per plan: downscaling delegates to downscale.ts
// (same module the client tests exercise) and result validation happens
// client-side (client.ts) via parseScanResult — this file only owns the
// wasm call and the wire protocol.
//
// Instantiated by the main thread as a module worker:
//   new Worker(new URL("./worker.ts", import.meta.url), { type: "module" })
// Vite bundles this file (and its imports, including the wasm package)
// for that worker entry automatically.

import init, { scan_rgba } from "../wasm/qrk_wasm.js";
import { downscaleRgba } from "./downscale";

interface ScanRequestMessage {
  type: "scan";
  id: number;
  rgba: ArrayBuffer;
  width: number;
  height: number;
  maxDim: number;
  withTrace: boolean;
}

interface ReadyMessage {
  type: "ready";
}

interface ScanResultMessage {
  type: "scan-result";
  id: number;
  ok: boolean;
  result?: unknown;
  error?: string;
  wallMs: number;
  scanWidth: number;
  scanHeight: number;
}

/** Just the worker-global surface this file touches — narrower than
 * `DedicatedWorkerGlobalScope` (which needs the `webworker` lib and
 * conflicts with this project's `DOM` lib) but precisely typed for what's
 * actually used here. */
interface WorkerGlobalLike {
  postMessage(message: ReadyMessage | ScanResultMessage): void;
  addEventListener(
    type: "message",
    listener: (ev: MessageEvent<ScanRequestMessage>) => void,
  ): void;
}

const ctx = self as unknown as WorkerGlobalLike;

const ready = init()
  .then(() => {
    ctx.postMessage({ type: "ready" });
  })
  .catch((err: unknown) => {
    // No dedicated "init failed" message in the protocol (out of scope for
    // this task) — surface it the same way a scan failure would so it's at
    // least visible in the console instead of silently hanging forever.
    console.error("scanner worker: wasm init failed", err);
  });

ctx.addEventListener("message", (ev) => {
  const msg = ev.data;
  if (msg.type !== "scan") return;

  void (async () => {
    await ready;

    const full = new Uint8ClampedArray(msg.rgba);
    const {
      rgba: scanRgba,
      width: scanWidth,
      height: scanHeight,
    } = downscaleRgba(full, msg.width, msg.height, msg.maxDim);
    // scan_rgba wants a Uint8Array view over the same bytes — no copy.
    const view = new Uint8Array(scanRgba.buffer, scanRgba.byteOffset, scanRgba.byteLength);

    try {
      const start = performance.now();
      const result: unknown = scan_rgba(view, scanWidth, scanHeight, msg.withTrace);
      const wallMs = performance.now() - start;
      ctx.postMessage({
        type: "scan-result",
        id: msg.id,
        ok: true,
        result,
        wallMs,
        scanWidth,
        scanHeight,
      });
    } catch (err) {
      ctx.postMessage({
        type: "scan-result",
        id: msg.id,
        ok: false,
        error: err instanceof Error ? err.message : String(err),
        wallMs: 0,
        scanWidth,
        scanHeight,
      });
    }
  })();
});
