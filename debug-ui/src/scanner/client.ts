// Main-thread client for the scanner Worker (worker.ts). All scanning runs
// off the main thread; this class owns the request/response protocol and
// "latest-wins" queueing so video mode can fire a scan per frame without
// building an unbounded backlog when frames arrive faster than scan_rgba
// can process them.

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
   * `qrk_core::ScanOptions::refine`'s doc): when `true`, each decoded
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

interface PendingScan {
  id: number;
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
  opts: ScanOptions;
  resolve: (outcome: ScanOutcome) => void;
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
      const pending: PendingScan = {
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
      };

      if (!this.inFlight) {
        this.start(pending);
        return;
      }

      this.queued?.reject(new StaleScanError());
      this.queued = pending;
    });
  }

  /** Releases the underlying worker. The client is unusable afterward. */
  dispose(): void {
    if (this.initTimeoutId != null) {
      clearTimeout(this.initTimeoutId);
      this.initTimeoutId = null;
    }
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
    const message: ScanRequestMessage = {
      type: "scan",
      id: pending.id,
      rgba: buffer,
      width: pending.width,
      height: pending.height,
      maxDim: pending.opts.maxDim,
      withTrace: pending.opts.withTrace,
      refine: pending.opts.refine ?? false,
    };
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
        const result = parseScanResult(data.result);
        pending.resolve({
          result,
          wallMs: data.wallMs,
          scanWidth: data.scanWidth,
          scanHeight: data.scanHeight,
        });
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
