// Scanner Web Worker: loads the qrk-wasm package once, then services scan
// requests off the main thread so a 24MP frame's detection cost never
// blocks the UI. Kept thin per plan: result validation happens client-side
// (client.ts) via parseScanResult/parseRobustScanResult — this file only
// owns the wasm calls and the wire protocol.
//
// Plan 5 Task 1: this file no longer downscales. `scan_rgba` now takes the
// SOURCE rgba + `maxDim` directly and owns the NN downscale in Rust (see
// `qrk_core::scan`/`downscale_luma`) — this worker just forwards the full
// frame it was sent. Worker traffic has ALWAYS carried the full source
// frame (App sends source rgba; pre-Task-1 the worker downscaled it after
// receipt) — Task 1 only moved WHERE the downscale computes (JS→Rust); a
// 24MP photo still transfers ~98MB per scan. Recorded/accepted cost:
// this is a dev tool, not the mobile production path (which receives a
// borrowed Y-plane and never goes through postMessage at all), and the
// copy-once ownership semantics (`ScannerClient.scan`'s doc comment)
// don't change — still exactly one copy per scan, just of more bytes.
// `downscale.ts` itself is unchanged and still used by `App.tsx`'s DISPLAY
// path (the bitmap drawn in the Viewport), which this worker has nothing
// to do with.
//
// Plan 6: a "scan-robust" request runs `scan_rgba_robust` (the adaptive
// escalation ladder) instead of `scan_rgba`; both answer with the SAME
// "scan-result" message shape (the client knows which parser to apply from
// its own pending-request bookkeeping, so the wire doesn't need a second
// response type). A one-shot "robust-presets" request serves
// `robust_presets()` — the authoritative Rust preset values.
//
// Plan 6 (session mode): the STATEFUL video path. A `WasmScanSession`
// amortizes the robust ladder across near-duplicate frames (rung rotation +
// cross-frame candidate pooling), so it must persist across frames and be
// reset on a scene change (source swap / seek). Three messages drive it:
//   - "session-config" (fire-and-forget): (re)builds the module-scoped
//     session with a `RobustConfig` + the two temporal knobs, dropping any
//     previous one (`.free()` to avoid a wasm leak).
//   - "session-reset" (fire-and-forget): clears cross-frame state.
//   - "scan-session-frame" (request/response): `session.scan_frame_rgba`,
//     answered with the SAME "scan-result" envelope `scan-robust` uses
//     (identical `WasmRobustResult` shape, so client parsing is unchanged).
//     Errors (ok:false) if no session was configured first.
//
// Instantiated by the main thread as a module worker:
//   new Worker(new URL("./worker.ts", import.meta.url), { type: "module" })
// Vite bundles this file (and its imports, including the wasm package)
// for that worker entry automatically.

import init, {
  robust_presets,
  scan_rgba,
  scan_rgba_robust,
  WasmScanSession,
} from "../wasm/qrk_wasm.js";

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

/** Plan 6: run the robust escalation ladder instead of the plain scan.
 * `config` is the camelCase `RobustConfig` object `scan_rgba_robust`
 * deserializes itself (forwarded opaquely — the worker doesn't validate
 * it; the wasm side rejects a malformed config with a real error). */
interface RobustScanRequestMessage {
  type: "scan-robust";
  id: number;
  rgba: ArrayBuffer;
  width: number;
  height: number;
  maxDim: number;
  refine: boolean;
  config: unknown;
  capture: boolean;
}

/** Plan 6: one-shot fetch of `robust_presets()` (the client caches it). */
interface RobustPresetsRequestMessage {
  type: "robust-presets";
  id: number;
}

/** Plan 6 (session): (re)build the module-scoped `WasmScanSession`. Sent
 * fire-and-forget (no `id`) — the next `scan-session-frame` observes the
 * new session; the ordering holds because both handlers `await ready` and
 * their continuations run in message-arrival order (config posted before
 * the first frame). `config` is forwarded opaquely, same as
 * `scan-robust`. */
interface SessionConfigMessage {
  type: "session-config";
  config: unknown;
  rotationPeriod: number;
  poolTtlFrames: number;
}

/** Plan 6 (session): drop the session's cross-frame state (source change /
 * seek). Fire-and-forget; no-op if no session exists yet. */
interface SessionResetMessage {
  type: "session-reset";
}

/** Plan 6 (session): scan one video frame through the persistent session.
 * Same rgba-transfer/latest-wins semantics as `scan`/`scan-robust`, and the
 * SAME "scan-result" response envelope (`WasmRobustResult` shape). */
interface ScanSessionFrameRequestMessage {
  type: "scan-session-frame";
  id: number;
  rgba: ArrayBuffer;
  width: number;
  height: number;
  maxDim: number;
  refine: boolean;
}

type RequestMessage =
  | ScanRequestMessage
  | RobustScanRequestMessage
  | RobustPresetsRequestMessage
  | SessionConfigMessage
  | SessionResetMessage
  | ScanSessionFrameRequestMessage;

/** Just the two fields this file reads off `scan_rgba`/`scan_rgba_robust`'s
 * return value before forwarding the whole (otherwise opaque) result to
 * the main thread — the working resolution `qrk_core::scan` actually
 * picked (see `qrk_wasm::WasmResult::scan_width`/`scan_height`); both
 * envelopes carry the same two fields. */
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

interface RobustPresetsResultMessage {
  type: "robust-presets-result";
  id: number;
  ok: boolean;
  presets?: unknown;
  error?: string;
}

/** Just the worker-global surface this file touches — narrower than
 * `DedicatedWorkerGlobalScope` (which needs the `webworker` lib and
 * conflicts with this project's `DOM` lib) but precisely typed for what's
 * actually used here. */
interface WorkerGlobalLike {
  postMessage(message: ReadyMessage | ScanResultMessage | RobustPresetsResultMessage): void;
  addEventListener(
    type: "message",
    listener: (ev: MessageEvent<RequestMessage>) => void,
  ): void;
}

const ctx = self as unknown as WorkerGlobalLike;

/** Plan 6 (session): the one persistent `WasmScanSession` for the video
 * path — stateful across frames (rung rotation + cross-frame candidate
 * pool). `null` until the first `session-config`; rebuilt (old one
 * `.free()`d) on every reconfigure. */
let session: WasmScanSession | null = null;

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

  if (msg.type === "robust-presets") {
    void (async () => {
      await ready;
      try {
        ctx.postMessage({
          type: "robust-presets-result",
          id: msg.id,
          ok: true,
          presets: robust_presets() as unknown,
        });
      } catch (err) {
        ctx.postMessage({
          type: "robust-presets-result",
          id: msg.id,
          ok: false,
          error: err instanceof Error ? err.message : String(err),
        });
      }
    })();
    return;
  }

  if (msg.type === "session-config") {
    // (Re)build the persistent session. Fire-and-forget: any error is
    // logged, not posted (there's no request id to answer, and the next
    // `scan-session-frame` surfaces a missing session as an ok:false result
    // anyway). Dropping the old session `.free()`s its wasm memory.
    void (async () => {
      await ready;
      try {
        if (session) {
          session.free();
          session = null;
        }
        session = new WasmScanSession(msg.config, msg.rotationPeriod, msg.poolTtlFrames);
      } catch (err) {
        console.error("scanner worker: session-config failed", err);
      }
    })();
    return;
  }

  if (msg.type === "session-reset") {
    void (async () => {
      await ready;
      try {
        session?.reset();
      } catch (err) {
        console.error("scanner worker: session-reset failed", err);
      }
    })();
    return;
  }

  if (msg.type !== "scan" && msg.type !== "scan-robust" && msg.type !== "scan-session-frame") return;

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

      // A session frame with no session configured is a client-side
      // ordering bug — surface it as an ok:false result (the catch below
      // posts it) rather than crashing the worker on `null.scan_frame_rgba`.
      if (msg.type === "scan-session-frame" && !session) {
        throw new Error(
          "scan-session-frame: no session configured (send session-config first)",
        );
      }

      const start = performance.now();
      let result: unknown;
      if (msg.type === "scan") {
        result = scan_rgba(view, msg.width, msg.height, msg.maxDim, msg.withTrace, msg.refine);
      } else if (msg.type === "scan-robust") {
        result = scan_rgba_robust(
          view,
          msg.width,
          msg.height,
          msg.maxDim,
          msg.refine,
          msg.config,
          msg.capture,
        );
      } else {
        // scan-session-frame — `session` is non-null (checked above).
        result = session!.scan_frame_rgba(view, msg.width, msg.height, msg.maxDim, msg.refine);
      }
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
