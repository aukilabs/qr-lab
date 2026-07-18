// Main-thread client for the scanner Worker (worker.ts). All scanning runs
// off the main thread; this class owns the request/response protocol and
// "latest-wins" queueing so video mode can fire a scan per frame without
// building an unbounded backlog when frames arrive faster than scan_rgba
// can process them.

import {
  parseRobustPresets,
  parseRobustScanResult,
  type RobustConfig,
  type RobustPresets,
  type RobustScanResult,
} from "./robust-types";
import { parseScanResult, type ScanResult } from "./types";

/**
 * The subset of the real `Worker` API this client depends on. Matching a
 * real `Worker` structurally (rather than importing the `Worker` type
 * directly) lets tests inject an in-memory fake instead of spinning up an
 * actual worker thread.
 */
export interface ScannerWorkerLike {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  addEventListener(type: "message", listener: (ev: MessageEvent) => void): void;
  removeEventListener(type: "message", listener: (ev: MessageEvent) => void): void;
  terminate(): void;
}

/** Thrown to reject a `scan()` call that was superseded by a newer one
 * before the worker ever started processing it. Video-mode callers should
 * treat this as "ignore — a fresher frame is already on its way", not as a
 * real failure. */
export class StaleScanError extends Error {
  constructor() {
    super("scan superseded by a newer request");
    this.name = "StaleScanError";
  }
}

/** How long `init()` waits for the worker's "ready" message before giving
 * up. See {@link WorkerInitTimeoutError}'s doc comment for why this exists. */
export const INIT_TIMEOUT_MS = 10_000;

/**
 * Thrown by {@link ScannerClient.init} when the worker never posts a
 * "ready" message within {@link INIT_TIMEOUT_MS}. Known gap this guards
 * against: `worker.ts`'s wasm `init()` failure is only `console.error`'d,
 * not surfaced as a message the client could reject on — so without this
 * timeout, a broken wasm build (missing/corrupt `.wasm` file, unsupported
 * browser, etc.) leaves `init()` pending forever with no feedback at all.
 * Callers (the debug UI's App shell) should catch this and show a banner
 * rather than leave the app looking like it's stuck loading.
 */
export class WorkerInitTimeoutError extends Error {
  constructor(timeoutMs: number) {
    super(
      `ScannerClient.init(): worker did not report ready within ${timeoutMs}ms — ` +
        `it likely failed during wasm init (check the worker's console output)`,
    );
    this.name = "WorkerInitTimeoutError";
  }
}

export interface ScanOptions {
  maxDim: number;
  withTrace: boolean;
  /** Enables subpixel corner refinement (Plan 5 Task 3 — see
   * `qr_lab_core::ScanOptions::refine`'s doc): when `true`, each decoded
   * code's `refined_corners` is populated (source px) instead of staying
   * `null`. Optional, defaulting to `false` in `start()` — media mode's
   * `App.tsx` passes `refine: true` explicitly (Plan 5 Task 7 QA fix), and
   * Scene3D hardcodes it the same way; a caller that omits this field
   * entirely still gets the `false` default. */
  refine?: boolean;
}

export interface ScanOutcome {
  result: ScanResult;
  wallMs: number;
  scanWidth: number;
  scanHeight: number;
}

/** Options for {@link ScannerClient.scanRobust} (Plan 6). No `withTrace` —
 * the robust envelope has no trace; the unified `detections` (finders/
 * triplets/codes across all rungs) always rides it, and the only extra
 * debug payload is the capture-gated filmstrip. */
export interface RobustScanOptions {
  maxDim: number;
  /** Same semantics as {@link ScanOptions.refine}; defaults to `false`. */
  refine?: boolean;
  /** camelCase flag object forwarded to `scan_rgba_robust` — see
   * `robust-types.ts`'s `RobustConfig` (source presets via
   * {@link ScannerClient.robustPresets}). */
  config: RobustConfig;
  /** `true` populates `snapshots` (the per-variant thumbnail filmstrip,
   * ~58 KB of pixels per rung) — the ONLY capture-gated payload; detection
   * results are identical either way. */
  capture: boolean;
}

export interface RobustScanOutcome {
  result: RobustScanResult;
  wallMs: number;
  scanWidth: number;
  scanHeight: number;
}

/** Options for {@link ScannerClient.scanSessionFrame} (Plan 6 session
 * mode). No `config`/`capture` here — the ladder config is baked into the
 * persistent session via {@link ScannerClient.configureSession}, and
 * capture (the filmstrip) is inherently OFF on the session video path. */
export interface SessionScanOptions {
  maxDim: number;
  /** Same semantics as {@link ScanOptions.refine}; defaults to `false`. */
  refine?: boolean;
}

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

interface RobustScanRequestMessage {
  type: "scan-robust";
  id: number;
  rgba: ArrayBuffer;
  width: number;
  height: number;
  maxDim: number;
  refine: boolean;
  config: RobustConfig;
  capture: boolean;
}

interface ScanSessionFrameRequestMessage {
  type: "scan-session-frame";
  id: number;
  rgba: ArrayBuffer;
  width: number;
  height: number;
  maxDim: number;
  refine: boolean;
}

interface PendingBase {
  id: number;
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
  reject: (err: unknown) => void;
}

/** Discriminated on `kind` so `handleMessage` knows which envelope parser
 * (and which resolve signature) a "scan-result" answer belongs to — plain
 * and robust requests share the SAME wire response type and the SAME
 * latest-wins slot (a newer request of EITHER kind supersedes a queued
 * one of either kind). */
type PendingScan =
  | (PendingBase & {
      kind: "scan";
      opts: ScanOptions;
      resolve: (outcome: ScanOutcome) => void;
    })
  | (PendingBase & {
      kind: "scan-robust";
      opts: RobustScanOptions;
      resolve: (outcome: RobustScanOutcome) => void;
    })
  | (PendingBase & {
      // Plan 6 session mode — shares the SAME latest-wins slot and the SAME
      // "scan-result" wire envelope (parsed by `parseRobustScanResult`, like
      // scan-robust); the persistent worker session is what makes it
      // stateful, not anything on this pending entry.
      kind: "session";
      opts: SessionScanOptions;
      resolve: (outcome: RobustScanOutcome) => void;
    });

interface PendingPresets {
  resolve: (presets: RobustPresets) => void;
  reject: (err: unknown) => void;
}

export class ScannerClient {
  private readonly worker: ScannerWorkerLike;
  private nextId = 1;
  private isReady = false;
  /** Latched true when the init timeout fires. Once set, a late "ready"
   * from the worker is ignored (see `handleMessage`) — `init()` already
   * rejected and told callers the client is permanently broken, so letting
   * a straggler ready flip `isReady` back on would resurrect a client the
   * caller has likely already replaced (or surfaced an error banner for),
   * with two clients then racing over one UI. */
  private initFailed = false;
  private readyResolve: (() => void) | null = null;
  private readyPromise: Promise<void> | null = null;
  private initTimeoutId: ReturnType<typeof setTimeout> | null = null;
  private inFlight: PendingScan | null = null;
  private queued: PendingScan | null = null;
  private pendingPresets = new Map<number, PendingPresets>();
  /** Cached `robustPresets()` result — the presets are compile-time Rust
   * constants, so one fetch per client is enough. Cleared on rejection so
   * a transient failure doesn't poison every later call. */
  private presetsPromise: Promise<RobustPresets> | null = null;

  constructor(worker: ScannerWorkerLike) {
    this.worker = worker;
    this.worker.addEventListener("message", this.handleMessage);
  }

  /** Resolves once the worker has finished loading wasm and is ready to
   * accept scan requests. Safe to call more than once (idempotent), and
   * safe even if the worker's "ready" message arrives before `init()` is
   * ever called — the ready state is latched independently of whether
   * anyone is awaiting it yet.
   *
   * Rejects with {@link WorkerInitTimeoutError} if no "ready" message
   * arrives within {@link INIT_TIMEOUT_MS} — see that error's doc comment
   * for why the timeout is necessary. Once rejected, the client stays
   * broken (this same rejection is returned by every subsequent `init()`
   * call); a caller that wants to retry should construct a fresh
   * `ScannerClient` around a fresh `Worker`. */
  async init(): Promise<void> {
    if (this.isReady) return;
    if (!this.readyPromise) {
      this.readyPromise = new Promise<void>((resolve, reject) => {
        this.readyResolve = resolve;
        this.initTimeoutId = setTimeout(() => {
          this.initTimeoutId = null;
          this.initFailed = true;
          reject(new WorkerInitTimeoutError(INIT_TIMEOUT_MS));
        }, INIT_TIMEOUT_MS);
      });
    }
    return this.readyPromise;
  }

  /**
   * Scan one RGBA frame. At most one scan runs on the worker at a time; if
   * a scan is already in flight, this request replaces any previously
   * queued (not-yet-started) request — that older queued request's promise
   * rejects immediately with {@link StaleScanError} rather than waiting
   * for the in-flight scan to finish.
   *
   * Ownership: `scan()` does NOT consume the caller's buffer. The frame is
   * copied exactly once, up front (`rgba.slice()` — copies just this
   * view's bytes into a fresh, whole-owned buffer), and it's the COPY's
   * buffer that later transfers to the worker — so the caller can keep
   * reading `rgba` after this returns (e.g. to downscale the same pixels
   * into a display bitmap) and can pass the same array to a later `scan()`
   * (re-scan / resolution change) without hitting a detached buffer. Cost:
   * one `rgba.byteLength`-byte copy per call. If a caller that never
   * reuses its frame ever needs to skip that copy, add an opt-in
   * `{ transfer: true }` option rather than changing this default —
   * App.tsx's display path depends on the caller keeping its bytes.
   */
  scan(
    rgba: Uint8ClampedArray,
    width: number,
    height: number,
    opts: ScanOptions,
  ): Promise<ScanOutcome> {
    return new Promise<ScanOutcome>((resolve, reject) => {
      this.enqueue({
        kind: "scan",
        id: this.nextId++,
        // The one copy per scan (see the ownership doc comment above).
        // Made here — not in start() — so a queued request holds its own
        // snapshot of the frame even if the caller mutates/reuses `rgba`
        // while an earlier scan is still in flight.
        rgba: rgba.slice(),
        width,
        height,
        opts,
        resolve,
        reject,
      });
    });
  }

  /**
   * Scan one RGBA frame through the Plan 6 robust escalation ladder
   * (`scan_rgba_robust`). Same ownership contract as {@link scan} (the
   * frame is copied once up front), and the same latest-wins slot: a
   * robust request and a plain request supersede each other — a newer
   * request of either kind replaces whichever kind is queued.
   */
  scanRobust(
    rgba: Uint8ClampedArray,
    width: number,
    height: number,
    opts: RobustScanOptions,
  ): Promise<RobustScanOutcome> {
    return new Promise<RobustScanOutcome>((resolve, reject) => {
      this.enqueue({
        kind: "scan-robust",
        id: this.nextId++,
        rgba: rgba.slice(), // same copy-once contract as scan() — see above
        width,
        height,
        opts,
        resolve,
        reject,
      });
    });
  }

  /**
   * (Re)build the worker's persistent {@link WasmScanSession} (Plan 6
   * session mode) with `config` and the two temporal knobs. Fire-and-forget
   * (no response): the next {@link scanSessionFrame} observes the new
   * session. Rebuilding drops the previous session's cross-frame state, so
   * calling this on a config/param change doubles as a reset.
   */
  configureSession(config: RobustConfig, rotationPeriod: number, poolTtlFrames: number): void {
    this.worker.postMessage({ type: "session-config", config, rotationPeriod, poolTtlFrames });
  }

  /**
   * Clear the worker session's cross-frame candidate pool without rebuilding
   * it (Plan 6) — call on a source change or a video seek, where a large
   * temporal jump invalidates candidates pooled from the previous scene.
   * Fire-and-forget; a no-op in the worker if no session exists yet.
   */
  resetSession(): void {
    this.worker.postMessage({ type: "session-reset" });
  }

  /**
   * Scan one video frame through the persistent session
   * (`WasmScanSession.scan_frame_rgba`, Plan 6). Same ownership contract as
   * {@link scan} (copied once up front) and the same latest-wins slot as
   * {@link scan}/{@link scanRobust} — a newer request of ANY kind supersedes
   * a queued one. The reply parses with `parseRobustScanResult` (identical
   * envelope to scan-robust; `snapshots` is always `null` — capture is off
   * on the session path). Requires {@link configureSession} to have been
   * called first, else the worker answers with an error.
   */
  scanSessionFrame(
    rgba: Uint8ClampedArray,
    width: number,
    height: number,
    opts: SessionScanOptions,
  ): Promise<RobustScanOutcome> {
    return new Promise<RobustScanOutcome>((resolve, reject) => {
      this.enqueue({
        kind: "session",
        id: this.nextId++,
        rgba: rgba.slice(), // same copy-once contract as scan() — see above
        width,
        height,
        opts,
        resolve,
        reject,
      });
    });
  }

  /**
   * Fetch the authoritative Rust `ScanConfig` presets
   * (`robust_presets()`), cached after the first successful call — the
   * values are compile-time constants, so every later call resolves from
   * the cache without touching the worker.
   */
  robustPresets(): Promise<RobustPresets> {
    if (!this.presetsPromise) {
      const promise = new Promise<RobustPresets>((resolve, reject) => {
        const id = this.nextId++;
        this.pendingPresets.set(id, { resolve, reject });
        this.worker.postMessage({ type: "robust-presets", id });
      });
      this.presetsPromise = promise;
      promise.catch(() => {
        // Allow a retry after a failure (see `presetsPromise`'s doc) —
        // but only if a newer fetch hasn't already replaced this one.
        if (this.presetsPromise === promise) this.presetsPromise = null;
      });
    }
    return this.presetsPromise;
  }

  private enqueue(pending: PendingScan): void {
    if (!this.inFlight) {
      this.start(pending);
      return;
    }
    this.queued?.reject(new StaleScanError());
    this.queued = pending;
  }

  /** Releases the underlying worker. The client is unusable afterward. */
  dispose(): void {
    if (this.initTimeoutId != null) {
      clearTimeout(this.initTimeoutId);
      this.initTimeoutId = null;
    }
    for (const pending of this.pendingPresets.values()) {
      pending.reject(new Error("ScannerClient disposed"));
    }
    this.pendingPresets.clear();
    this.worker.removeEventListener("message", this.handleMessage);
    this.worker.terminate();
  }

  private start(pending: PendingScan): void {
    this.inFlight = pending;
    // `pending.rgba` is always the private copy `scan()` made (a fresh
    // `slice()`, so it owns its whole buffer with byteOffset 0 — even when
    // the caller passed a subarray view of a larger buffer). Transferring
    // its buffer is therefore exact (just this frame's bytes) and detaches
    // only the copy, never anything the caller holds.
    const buffer = pending.rgba.buffer as ArrayBuffer;
    let message: ScanRequestMessage | RobustScanRequestMessage | ScanSessionFrameRequestMessage;
    if (pending.kind === "scan") {
      message = {
        type: "scan",
        id: pending.id,
        rgba: buffer,
        width: pending.width,
        height: pending.height,
        maxDim: pending.opts.maxDim,
        withTrace: pending.opts.withTrace,
        refine: pending.opts.refine ?? false,
      };
    } else if (pending.kind === "scan-robust") {
      message = {
        type: "scan-robust",
        id: pending.id,
        rgba: buffer,
        width: pending.width,
        height: pending.height,
        maxDim: pending.opts.maxDim,
        refine: pending.opts.refine ?? false,
        config: pending.opts.config,
        capture: pending.opts.capture,
      };
    } else {
      message = {
        type: "scan-session-frame",
        id: pending.id,
        rgba: buffer,
        width: pending.width,
        height: pending.height,
        maxDim: pending.opts.maxDim,
        refine: pending.opts.refine ?? false,
      };
    }
    this.worker.postMessage(message, [buffer]);
  }

  private readonly handleMessage = (ev: MessageEvent): void => {
    const msg = ev.data as { type?: unknown };
    if (msg?.type === "ready") {
      if (this.initFailed) return; // too late — init() already rejected; see `initFailed`
      this.isReady = true;
      if (this.initTimeoutId != null) {
        clearTimeout(this.initTimeoutId);
        this.initTimeoutId = null;
      }
      this.readyResolve?.();
      return;
    }
    if (msg?.type === "robust-presets-result") {
      const data = msg as { id: number; ok: boolean; presets?: unknown; error?: string };
      const pending = this.pendingPresets.get(data.id);
      if (!pending) return;
      this.pendingPresets.delete(data.id);
      if (data.ok) {
        try {
          pending.resolve(parseRobustPresets(data.presets));
        } catch (err) {
          pending.reject(err);
        }
      } else {
        pending.reject(new Error(data.error ?? "robust_presets failed"));
      }
      return;
    }
    if (msg?.type !== "scan-result") return;

    const data = msg as {
      id: number;
      ok: boolean;
      result?: unknown;
      error?: string;
      wallMs: number;
      scanWidth: number;
      scanHeight: number;
    };

    const pending = this.inFlight;
    if (!pending || pending.id !== data.id) return; // stale/unknown — ignore

    this.inFlight = null;
    if (data.ok) {
      try {
        // The wire response is shared across all scan kinds; the pending
        // entry's own `kind` picks the right envelope parser. `scan-robust`
        // and `session` both carry a `WasmRobustResult` and resolve a
        // `RobustScanOutcome`, so they share the else branch.
        if (pending.kind === "scan") {
          pending.resolve({
            result: parseScanResult(data.result),
            wallMs: data.wallMs,
            scanWidth: data.scanWidth,
            scanHeight: data.scanHeight,
          });
        } else {
          pending.resolve({
            result: parseRobustScanResult(data.result),
            wallMs: data.wallMs,
            scanWidth: data.scanWidth,
            scanHeight: data.scanHeight,
          });
        }
      } catch (err) {
        pending.reject(err);
      }
    } else {
      pending.reject(new Error(data.error ?? "scan failed"));
    }

    const next = this.queued;
    this.queued = null;
    if (next) this.start(next);
  };
}
