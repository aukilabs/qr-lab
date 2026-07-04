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
        rgba,
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
    const { rgba } = pending;
    // Transferring rgba.buffer directly hands the worker the *entire*
    // underlying ArrayBuffer, not just this view's slice of it. That's
    // correct (and zero-copy) for the common case — a typed array that
    // owns its whole buffer — but would silently ship extra/wrong bytes if
    // `rgba` were ever a subarray view (nonzero byteOffset or a shorter
    // byteLength), which the worker has no way to detect from a bare
    // ArrayBuffer. Copy only the view's bytes in that case.
    const buffer = (
      rgba.byteOffset === 0 && rgba.byteLength === rgba.buffer.byteLength
        ? rgba.buffer
        : rgba.buffer.slice(rgba.byteOffset, rgba.byteOffset + rgba.byteLength)
    ) as ArrayBuffer;
    const message: ScanRequestMessage = {
      type: "scan",
      id: pending.id,
      rgba: buffer,
      width: pending.width,
      height: pending.height,
      maxDim: pending.opts.maxDim,
      withTrace: pending.opts.withTrace,
    };
    this.worker.postMessage(message, [buffer]);
  }

  private readonly handleMessage = (ev: MessageEvent): void => {
    const msg = ev.data as { type?: unknown };
    if (msg?.type === "ready") {
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
