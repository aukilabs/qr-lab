// Scanner Web Worker: loads the qrk-wasm package once, then services scan
// requests off the main thread so a 24MP frame's detection cost never
// blocks the UI. Kept thin per plan: result validation happens client-side
// (client.ts) via parseScanResult — this file only owns the wasm call and
// the wire protocol.
//
// Plan 5 Task 1: this file no longer downscales. `scan_rgba` now takes the
// SOURCE rgba + `maxDim` directly and owns the NN downscale in Rust (see
// `qrk_core::scan`/`downscale_luma`) — this worker just forwards the full
// frame it was sent. That means worker traffic (the `postMessage` transfer
// from the main thread) now carries the FULL source frame instead of an
// already-downscaled one — a 24MP photo transfers ~98MB instead of a few
// MB. Recorded/accepted cost (Plan 5 Global Constraints "Known risks"):
// this is a dev tool, not the mobile production path (which receives a
// borrowed Y-plane and never goes through postMessage at all), and the
// copy-once ownership semantics (`ScannerClient.scan`'s doc comment)
// don't change — still exactly one copy per scan, just of more bytes.
// `downscale.ts` itself is unchanged and still used by `App.tsx`'s DISPLAY
// path (the bitmap drawn in the Viewport), which this worker has nothing
// to do with.
//
// Instantiated by the main thread as a module worker:
//   new Worker(new URL("./worker.ts", import.meta.url), { type: "module" })
// Vite bundles this file (and its imports, including the wasm package)
// for that worker entry automatically.

import init, { scan_rgba } from "../wasm/qrk_wasm.js";

interface ScanRequestMessage {
  type: "scan";
  id: number;
  rgba: ArrayBuffer;
  width: number;
  height: number;
  maxDim: number;
  withTrace: boolean;
  refine: boolean;
}

/** Just the two fields this file reads off `scan_rgba`'s return value
 * before forwarding the whole (otherwise opaque) result to the main
 * thread — the working resolution `qrk_core::scan` actually picked (see
 * `qrk_wasm::WasmResult::scan_width`/`scan_height`), now Rust-computed
 * instead of derived from a local `downscaleRgba` call. */
interface ScanRgbaResult {
  scan_width: number;
  scan_height: number;
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

    // Hoisted with the requested (source) dims as a fallback: if
    // constructing `view`/the wasm call throws before the real working
    // `scanWidth`/`scanHeight` are known (from the wasm result itself —
    // see `ScanRgbaResult` above), the catch below still has *something*
    // dimension-shaped to report instead of a ReferenceError — which
    // would propagate out of this async IIFE as an unhandled rejection
    // instead of posting a `scan-result`, leaving the client's `inFlight`
    // entry for `msg.id` never resolved (permanently "stale" for every
    // scan after it). Everything that can throw — constructing the typed
    // array and the wasm call itself — now runs inside the try below so a
    // scan-result (ok or not) always gets posted.
    let scanWidth = msg.width;
    let scanHeight = msg.height;

    try {
      // scan_rgba wants a Uint8Array view over the SOURCE rgba's bytes —
      // no copy, and no local downscale: `qrk_core::scan` owns that now
      // (see this file's module doc comment).
      const full = new Uint8ClampedArray(msg.rgba);
      const view = new Uint8Array(full.buffer, full.byteOffset, full.byteLength);

      const start = performance.now();
      const result: unknown = scan_rgba(
        view,
        msg.width,
        msg.height,
        msg.maxDim,
        msg.withTrace,
        msg.refine,
      );
      const wallMs = performance.now() - start;
      const scanned = result as ScanRgbaResult;
      scanWidth = scanned.scan_width;
      scanHeight = scanned.scan_height;
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
