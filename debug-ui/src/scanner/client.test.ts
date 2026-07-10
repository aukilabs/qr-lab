import { afterEach, describe, expect, it, vi } from "vitest";
import {
  INIT_TIMEOUT_MS,
  ScannerClient,
  StaleScanError,
  WorkerInitTimeoutError,
  type ScannerWorkerLike,
} from "./client";
import { ScanResultParseError } from "./types";

/** Minimal in-memory stand-in for the real Worker, driven manually by
 * tests: `emit()` synchronously dispatches a fake `MessageEvent` to
 * whatever listener ScannerClient registered, so tests control exactly
 * when (and in what order) "worker" responses arrive.
 *
 * `postMessage` simulates the structured-clone TRANSFER semantics the real
 * Worker has: every `ArrayBuffer` in the transfer list is detached on the
 * "sending" side via `ArrayBuffer.prototype.transfer()` (the message's
 * `rgba` field is swapped to the moved — still readable — buffer, mirroring
 * what the receiving context would see). Without this, a bug where the
 * client transfers a buffer the caller still needs would be invisible to
 * these tests — the caller's view would silently stay intact in the fake
 * while detaching for real in a browser. */
class FakeWorker implements ScannerWorkerLike {
  posted: Array<{ message: any; transfer?: Transferable[] }> = [];
  terminated = false;
  private listeners: Array<(ev: MessageEvent) => void> = [];

  postMessage(message: unknown, transfer?: Transferable[]): void {
    if (transfer) {
      const msg = message as { rgba?: ArrayBuffer };
      for (const t of transfer) {
        if (!(t instanceof ArrayBuffer)) continue;
        // `.transfer()` detaches `t` exactly like a real postMessage
        // would. It's an ES2024 API (supported by the Node this runs on)
        // that this project's ES2022 `lib` doesn't type yet — cast locally
        // rather than widening the whole project's lib for one test fake.
        const moved = (t as ArrayBuffer & { transfer(): ArrayBuffer }).transfer();
        if (msg.rgba === t) msg.rgba = moved;
      }
    }
    this.posted.push(transfer === undefined ? { message } : { message, transfer });
  }

  addEventListener(type: "message", listener: (ev: MessageEvent) => void): void {
    if (type === "message") this.listeners.push(listener);
  }

  removeEventListener(type: "message", listener: (ev: MessageEvent) => void): void {
    if (type !== "message") return;
    this.listeners = this.listeners.filter((l) => l !== listener);
  }

  terminate(): void {
    this.terminated = true;
  }

  emit(data: unknown): void {
    for (const listener of this.listeners) {
      listener({ data } as MessageEvent);
    }
  }
}

function minimalScanResultJson(): unknown {
  return {
    detections: {
      finders: [],
      triplets: [],
      codes: [],
      timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
        refine_ns: 0,
      },
      source_scale: 1,
    },
    trace: null,
  };
}

function makeRgba(len: number): Uint8ClampedArray {
  return new Uint8ClampedArray(len);
}

// --- Plan 6 robust-mode fixtures ---

function minimalRobustResultJson(): unknown {
  return {
    robust: {
      codes: [],
      variants: [
        {
          kind: "Baseline",
          stage: 0,
          timings: {
            tiles_ns: 0,
            finders_ns: 0,
            triplets_ns: 0,
            version_ns: 0,
            alignment_ns: 0,
            sample_decode_ns: 0,
            refine_ns: 0,
          },
          total_ns: 0,
          finders: 0,
          triplets: 0,
          codes: 0,
          new_codes: 0,
        },
      ],
      early_exited: false,
      budget_exhausted: false,
      total_ns: 0,
      triplet_evidence: [],
    },
    // The unified pipeline detections — one-pipeline contract: same shape
    // as the classic envelope's `detections`, always present.
    detections: {
      finders: [],
      triplets: [],
      codes: [],
      timings: {
        tiles_ns: 0,
        finders_ns: 0,
        triplets_ns: 0,
        version_ns: 0,
        alignment_ns: 0,
        sample_decode_ns: 0,
        refine_ns: 0,
      },
      source_scale: 1,
    },
    // The real binding's capture-off shape: key present, value undefined
    // (serde-wasm-bindgen's Option::None) — exactly what the parser must
    // accept alongside JSON's null.
    snapshots: undefined,
    scan_width: 2,
    scan_height: 2,
  };
}

function allOffConfig() {
  return {
    enableMultiScale: false,
    enableContrastNormalization: false,
    enableShadowNormalization: false,
    enableAdaptiveThresholding: false,
    enableSharpening: false,
    enableDeblur: false,
    enableLowResUpscaling: false,
    maxVariantsPerFrame: 0,
    enableEarlyExit: false,
  };
}

describe("ScannerClient", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  describe("init() timeout", () => {
    it("rejects with WorkerInitTimeoutError when no ready message arrives in time", async () => {
      vi.useFakeTimers();
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);

      const initPromise = client.init();
      const onSettle = vi.fn();
      initPromise.then(onSettle, onSettle);

      await vi.advanceTimersByTimeAsync(INIT_TIMEOUT_MS - 1);
      expect(onSettle).not.toHaveBeenCalled();

      await vi.advanceTimersByTimeAsync(1);
      await expect(initPromise).rejects.toBeInstanceOf(WorkerInitTimeoutError);
      await expect(initPromise).rejects.toThrow(/did not report ready/);
    });

    it("does not time out if ready arrives before the deadline", async () => {
      vi.useFakeTimers();
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);

      const initPromise = client.init();
      await vi.advanceTimersByTimeAsync(INIT_TIMEOUT_MS - 1);
      worker.emit({ type: "ready" });
      await expect(initPromise).resolves.toBeUndefined();

      // No pending timer left dangling once ready has landed.
      await vi.advanceTimersByTimeAsync(INIT_TIMEOUT_MS * 2);
    });

    it("a second init() call after a timeout returns the same rejection", async () => {
      vi.useFakeTimers();
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);

      // Attach the rejection assertion synchronously (before advancing
      // timers) so the promise never sits unhandled for a tick — Node
      // flags that as a (harmless here, but noisy) unhandled-rejection
      // warning even though it's `await`ed a few lines later.
      const first = client.init();
      const firstRejection = expect(first).rejects.toBeInstanceOf(WorkerInitTimeoutError);
      await vi.advanceTimersByTimeAsync(INIT_TIMEOUT_MS);
      await firstRejection;

      await expect(client.init()).rejects.toBeInstanceOf(WorkerInitTimeoutError);
    });

    it("a late ready arriving after the timeout does not resurrect the failed client", async () => {
      vi.useFakeTimers();
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);

      const first = client.init();
      const firstRejection = expect(first).rejects.toBeInstanceOf(WorkerInitTimeoutError);
      await vi.advanceTimersByTimeAsync(INIT_TIMEOUT_MS);
      await firstRejection;

      // Worker limps to life AFTER init() already rejected — the client is
      // documented as permanently failed at this point (callers were told
      // to build a fresh client to retry), so the late ready must be
      // ignored: a second init() still rejects rather than resolving.
      worker.emit({ type: "ready" });
      await expect(client.init()).rejects.toBeInstanceOf(WorkerInitTimeoutError);
    });
  });

  it("init() resolves once the worker posts a ready message", async () => {
    const worker = new FakeWorker();
    const client = new ScannerClient(worker);

    let resolved = false;
    const initPromise = client.init().then(() => {
      resolved = true;
    });
    expect(resolved).toBe(false);

    worker.emit({ type: "ready" });
    await initPromise;
    expect(resolved).toBe(true);
  });

  it("posts a scan request and resolves with the parsed result", async () => {
    const worker = new FakeWorker();
    const client = new ScannerClient(worker);
    worker.emit({ type: "ready" });
    await client.init();

    const rgba = makeRgba(4 * 4 * 4);
    const scanPromise = client.scan(rgba, 4, 4, { maxDim: 0, withTrace: false });

    expect(worker.posted).toHaveLength(1);
    const req = worker.posted[0]!;
    expect(req.message).toMatchObject({
      type: "scan",
      width: 4,
      height: 4,
      maxDim: 0,
      withTrace: false,
    });
    expect(req.message.rgba).toBeInstanceOf(ArrayBuffer);
    expect(req.transfer).toEqual([req.message.rgba]);

    worker.emit({
      type: "scan-result",
      id: req.message.id,
      ok: true,
      result: minimalScanResultJson(),
      wallMs: 12.5,
      scanWidth: 4,
      scanHeight: 4,
    });

    const outcome = await scanPromise;
    expect(outcome.wallMs).toBe(12.5);
    expect(outcome.scanWidth).toBe(4);
    expect(outcome.scanHeight).toBe(4);
    expect(outcome.result.detections.finders).toEqual([]);
    expect(outcome.result.trace).toBeNull();
  });

  it("transfers only the view's bytes when rgba is a subarray of a larger buffer", async () => {
    const worker = new FakeWorker();
    const client = new ScannerClient(worker);
    worker.emit({ type: "ready" });
    await client.init();

    // A 2x2 RGBA view (16 bytes) living in the middle of a much larger
    // buffer, at a nonzero byteOffset — e.g. one tile of a shared frame
    // buffer. Mark the view's bytes distinctly from the surrounding noise
    // so a wrong offset/length would be caught even if byteLength matched
    // by coincidence.
    const backing = new Uint8ClampedArray(64).fill(0xaa);
    const view = new Uint8ClampedArray(backing.buffer, 20, 2 * 2 * 4);
    view.fill(0x42);

    client.scan(view, 2, 2, { maxDim: 0, withTrace: false });

    const req = worker.posted[0]!;
    const sent = new Uint8Array(req.message.rgba as ArrayBuffer);
    expect(sent.byteLength).toBe(16);
    expect(Array.from(sent)).toEqual(new Array(16).fill(0x42));
    expect(req.transfer).toEqual([req.message.rgba]);
  });

  it("does not detach the caller's buffer — the same rgba stays readable and re-scannable", async () => {
    const worker = new FakeWorker();
    const client = new ScannerClient(worker);
    worker.emit({ type: "ready" });
    await client.init();

    const rgba = makeRgba(2 * 2 * 4);
    rgba.fill(0x42);

    const first = client.scan(rgba, 2, 2, { maxDim: 0, withTrace: false });

    // The transferred buffer must be a COPY, never the caller's own buffer
    // (FakeWorker.postMessage detaches whatever was in the transfer list,
    // so a violation shows up both as identity equality and as the
    // caller's view detaching to byteLength 0).
    const firstReq = worker.posted[0]!;
    expect(firstReq.message.rgba).not.toBe(rgba.buffer);
    expect(rgba.byteLength).toBe(16);
    expect(Array.from(rgba)).toEqual(new Array(16).fill(0x42));
    // ...and the copy carries the right bytes.
    expect(Array.from(new Uint8Array(firstReq.message.rgba as ArrayBuffer))).toEqual(
      new Array(16).fill(0x42),
    );

    worker.emit({
      type: "scan-result",
      id: firstReq.message.id,
      ok: true,
      result: minimalScanResultJson(),
      wallMs: 1,
      scanWidth: 2,
      scanHeight: 2,
    });
    await first;

    // Re-scan with the SAME array (App's re-scan button / resolution
    // change path) — must not throw "already detached" and must ship the
    // same bytes again.
    client.scan(rgba, 2, 2, { maxDim: 0, withTrace: false });
    const secondReq = worker.posted[1]!;
    expect(Array.from(new Uint8Array(secondReq.message.rgba as ArrayBuffer))).toEqual(
      new Array(16).fill(0x42),
    );
    expect(rgba.byteLength).toBe(16);
  });

  it("rejects with the worker-reported error on ok: false", async () => {
    const worker = new FakeWorker();
    const client = new ScannerClient(worker);
    worker.emit({ type: "ready" });
    await client.init();

    const scanPromise = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
    const id = worker.posted[0]!.message.id;
    worker.emit({
      type: "scan-result",
      id,
      ok: false,
      error: "scan_rgba: boom",
      wallMs: 1,
      scanWidth: 2,
      scanHeight: 2,
    });

    await expect(scanPromise).rejects.toThrow(/scan_rgba: boom/);
  });

  it("rejects with ScanResultParseError when the worker's result fails validation", async () => {
    const worker = new FakeWorker();
    const client = new ScannerClient(worker);
    worker.emit({ type: "ready" });
    await client.init();

    const scanPromise = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
    const id = worker.posted[0]!.message.id;
    worker.emit({
      type: "scan-result",
      id,
      ok: true,
      result: { detections: {} }, // missing required fields
      wallMs: 1,
      scanWidth: 2,
      scanHeight: 2,
    });

    await expect(scanPromise).rejects.toBeInstanceOf(ScanResultParseError);
  });

  it("ignores scan-result messages whose id doesn't match the in-flight request", async () => {
    const worker = new FakeWorker();
    const client = new ScannerClient(worker);
    worker.emit({ type: "ready" });
    await client.init();

    const scanPromise = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
    const onSettle = vi.fn();
    scanPromise.then(onSettle, onSettle);

    // Stale/unknown id: must not settle the in-flight promise.
    worker.emit({
      type: "scan-result",
      id: 999999,
      ok: true,
      result: minimalScanResultJson(),
      wallMs: 1,
      scanWidth: 2,
      scanHeight: 2,
    });
    await Promise.resolve();
    await Promise.resolve();
    expect(onSettle).not.toHaveBeenCalled();

    const realId = worker.posted[0]!.message.id;
    worker.emit({
      type: "scan-result",
      id: realId,
      ok: true,
      result: minimalScanResultJson(),
      wallMs: 1,
      scanWidth: 2,
      scanHeight: 2,
    });
    await scanPromise;
    expect(onSettle).toHaveBeenCalledTimes(1);
  });

  describe("latest-wins queueing", () => {
    it("keeps at most one queued request, rejecting a superseded one with StaleScanError", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      const first = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      // Only the first scan should have been posted to the worker so far.
      expect(worker.posted).toHaveLength(1);

      const second = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      // Second is queued behind the in-flight first — not posted yet.
      expect(worker.posted).toHaveLength(1);

      const third = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      // Third supersedes the still-queued second before it ever started.
      await expect(second).rejects.toBeInstanceOf(StaleScanError);
      expect(worker.posted).toHaveLength(1);

      // Completing the in-flight first request starts the newest queued
      // request (third), skipping the superseded second entirely.
      const firstId = worker.posted[0]!.message.id;
      worker.emit({
        type: "scan-result",
        id: firstId,
        ok: true,
        result: minimalScanResultJson(),
        wallMs: 1,
        scanWidth: 2,
        scanHeight: 2,
      });
      await first;

      expect(worker.posted).toHaveLength(2);
      const thirdId = worker.posted[1]!.message.id;
      expect(thirdId).not.toBe(firstId);

      worker.emit({
        type: "scan-result",
        id: thirdId,
        ok: true,
        result: minimalScanResultJson(),
        wallMs: 2,
        scanWidth: 2,
        scanHeight: 2,
      });
      await expect(third).resolves.toMatchObject({ wallMs: 2 });
    });

    it("runs a scan immediately when nothing is in flight", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      expect(worker.posted).toHaveLength(1);
    });
  });

  // --- Plan 6: scanRobust + robustPresets ---

  describe("scanRobust", () => {
    it("posts a scan-robust request (config + capture on the wire) and resolves the parsed robust outcome", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      const config = allOffConfig();
      const scanPromise = client.scanRobust(makeRgba(16), 2, 2, {
        maxDim: 640,
        refine: true,
        config,
        capture: true,
      });

      expect(worker.posted).toHaveLength(1);
      const req = worker.posted[0]!;
      expect(req.message).toMatchObject({
        type: "scan-robust",
        width: 2,
        height: 2,
        maxDim: 640,
        refine: true,
        config,
        capture: true,
      });
      expect(req.message.rgba).toBeInstanceOf(ArrayBuffer);
      expect(req.transfer).toEqual([req.message.rgba]);

      worker.emit({
        type: "scan-result",
        id: req.message.id,
        ok: true,
        result: minimalRobustResultJson(),
        wallMs: 3,
        scanWidth: 2,
        scanHeight: 2,
      });

      const outcome = await scanPromise;
      expect(outcome.result.robust.variants).toHaveLength(1);
      expect(outcome.result.robust.variants[0]?.kind).toBe("Baseline");
      // The unified detections always parse (one-pipeline contract)...
      expect(outcome.result.detections.codes).toEqual([]);
      // ...and the undefined-valued optional filmstrip (the real binding's
      // capture-off shape) parsed to null.
      expect(outcome.result.snapshots).toBeNull();
    });

    it("rejects with ScanResultParseError when the robust result fails validation", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      const scanPromise = client.scanRobust(makeRgba(16), 2, 2, {
        maxDim: 0,
        config: allOffConfig(),
        capture: false,
      });
      worker.emit({
        type: "scan-result",
        id: worker.posted[0]!.message.id,
        ok: true,
        result: { robust: {} }, // missing required fields
        wallMs: 1,
        scanWidth: 2,
        scanHeight: 2,
      });

      await expect(scanPromise).rejects.toBeInstanceOf(ScanResultParseError);
    });

    it("shares the latest-wins slot with plain scans — a robust request supersedes a queued plain one and vice versa", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      // Plain scan in flight, plain scan queued, then a ROBUST request
      // arrives: the queued plain scan must reject stale.
      const first = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      const queuedPlain = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      const robust = client.scanRobust(makeRgba(16), 2, 2, {
        maxDim: 0,
        config: allOffConfig(),
        capture: false,
      });
      await expect(queuedPlain).rejects.toBeInstanceOf(StaleScanError);
      expect(worker.posted).toHaveLength(1);

      // And the reverse: a newer PLAIN request supersedes the queued
      // robust one.
      const newestPlain = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      await expect(robust).rejects.toBeInstanceOf(StaleScanError);

      // Completing the in-flight first starts the newest request only.
      worker.emit({
        type: "scan-result",
        id: worker.posted[0]!.message.id,
        ok: true,
        result: minimalScanResultJson(),
        wallMs: 1,
        scanWidth: 2,
        scanHeight: 2,
      });
      await first;
      expect(worker.posted).toHaveLength(2);
      expect(worker.posted[1]!.message.type).toBe("scan");

      worker.emit({
        type: "scan-result",
        id: worker.posted[1]!.message.id,
        ok: true,
        result: minimalScanResultJson(),
        wallMs: 2,
        scanWidth: 2,
        scanHeight: 2,
      });
      await expect(newestPlain).resolves.toMatchObject({ wallMs: 2 });
    });
  });

  // --- Plan 6: session mode (configureSession / resetSession / scanSessionFrame) ---

  describe("session mode", () => {
    it("configureSession posts a session-config message with the config and both temporal knobs", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      const config = { ...allOffConfig(), enableMultiScale: true };
      client.configureSession(config, 5, 7);

      expect(worker.posted).toHaveLength(1);
      expect(worker.posted[0]!.message).toEqual({
        type: "session-config",
        config,
        rotationPeriod: 5,
        poolTtlFrames: 7,
      });
      // Fire-and-forget — no transfer list, nothing to await.
      expect(worker.posted[0]!.transfer).toBeUndefined();
    });

    it("resetSession posts a session-reset message", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      client.resetSession();
      expect(worker.posted).toHaveLength(1);
      expect(worker.posted[0]!.message).toEqual({ type: "session-reset" });
    });

    it("posts a scan-session-frame request (transferring the frame) and parses the robust outcome", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      const scanPromise = client.scanSessionFrame(makeRgba(16), 2, 2, { maxDim: 640, refine: true });

      expect(worker.posted).toHaveLength(1);
      const req = worker.posted[0]!;
      expect(req.message).toMatchObject({
        type: "scan-session-frame",
        width: 2,
        height: 2,
        maxDim: 640,
        refine: true,
      });
      // No config/capture on the wire — those live on the persistent session.
      expect(req.message.config).toBeUndefined();
      expect(req.message.capture).toBeUndefined();
      expect(req.message.rgba).toBeInstanceOf(ArrayBuffer);
      expect(req.transfer).toEqual([req.message.rgba]);

      worker.emit({
        type: "scan-result",
        id: req.message.id,
        ok: true,
        result: minimalRobustResultJson(),
        wallMs: 4,
        scanWidth: 2,
        scanHeight: 2,
      });

      const outcome = await scanPromise;
      expect(outcome.wallMs).toBe(4);
      // Parsed with the SAME robust parser as scanRobust (identical envelope).
      expect(outcome.result.robust.variants).toHaveLength(1);
      expect(outcome.result.robust.variants[0]?.kind).toBe("Baseline");
      // Capture is off on the session path — the undefined filmstrip parses to null.
      expect(outcome.result.snapshots).toBeNull();
    });

    it("defaults refine to false when omitted", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      client.scanSessionFrame(makeRgba(16), 2, 2, { maxDim: 0 });
      expect(worker.posted[0]!.message).toMatchObject({ type: "scan-session-frame", refine: false });
    });

    it("rejects with ScanResultParseError when the session result fails validation", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      const scanPromise = client.scanSessionFrame(makeRgba(16), 2, 2, { maxDim: 0 });
      worker.emit({
        type: "scan-result",
        id: worker.posted[0]!.message.id,
        ok: true,
        result: { robust: {} }, // missing required fields
        wallMs: 1,
        scanWidth: 2,
        scanHeight: 2,
      });

      await expect(scanPromise).rejects.toBeInstanceOf(ScanResultParseError);
    });

    it("shares the latest-wins slot with the other kinds — session supersedes a queued plain/robust and vice versa", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);
      worker.emit({ type: "ready" });
      await client.init();

      // Plain in flight, robust queued, then a SESSION frame arrives: the
      // queued robust must reject stale.
      const first = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      const queuedRobust = client.scanRobust(makeRgba(16), 2, 2, {
        maxDim: 0,
        config: allOffConfig(),
        capture: false,
      });
      const session = client.scanSessionFrame(makeRgba(16), 2, 2, { maxDim: 0 });
      await expect(queuedRobust).rejects.toBeInstanceOf(StaleScanError);
      expect(worker.posted).toHaveLength(1);

      // And the reverse: a newer PLAIN request supersedes the queued session.
      const newestPlain = client.scan(makeRgba(16), 2, 2, { maxDim: 0, withTrace: false });
      await expect(session).rejects.toBeInstanceOf(StaleScanError);

      // Completing the in-flight first starts only the newest (plain) request.
      worker.emit({
        type: "scan-result",
        id: worker.posted[0]!.message.id,
        ok: true,
        result: minimalScanResultJson(),
        wallMs: 1,
        scanWidth: 2,
        scanHeight: 2,
      });
      await first;
      expect(worker.posted).toHaveLength(2);
      expect(worker.posted[1]!.message.type).toBe("scan");

      worker.emit({
        type: "scan-result",
        id: worker.posted[1]!.message.id,
        ok: true,
        result: minimalScanResultJson(),
        wallMs: 2,
        scanWidth: 2,
        scanHeight: 2,
      });
      await expect(newestPlain).resolves.toMatchObject({ wallMs: 2 });
    });
  });

  describe("robustPresets", () => {
    const presetsJson = () => ({
      baseline: allOffConfig(),
      robustFast: { ...allOffConfig(), enableMultiScale: true, enableEarlyExit: true },
      robustFullBenchmark: { ...allOffConfig(), enableDeblur: true },
    });

    it("posts one robust-presets request and caches the parsed result for later calls", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);

      const firstCall = client.robustPresets();
      expect(worker.posted).toHaveLength(1);
      expect(worker.posted[0]!.message).toMatchObject({ type: "robust-presets" });

      worker.emit({
        type: "robust-presets-result",
        id: worker.posted[0]!.message.id,
        ok: true,
        presets: presetsJson(),
      });

      const presets = await firstCall;
      expect(presets.robustFast.enableMultiScale).toBe(true);
      expect(presets.baseline.enableMultiScale).toBe(false);

      // Cached: no second wire request, same resolved value.
      const secondCall = await client.robustPresets();
      expect(worker.posted).toHaveLength(1);
      expect(secondCall).toBe(presets);
    });

    it("rejects on a worker-reported error and allows a retry (cache cleared)", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);

      const firstCall = client.robustPresets();
      worker.emit({
        type: "robust-presets-result",
        id: worker.posted[0]!.message.id,
        ok: false,
        error: "boom",
      });
      await expect(firstCall).rejects.toThrow(/boom/);

      // Retry issues a fresh wire request rather than returning the
      // poisoned cache entry.
      const retry = client.robustPresets();
      expect(worker.posted).toHaveLength(2);
      worker.emit({
        type: "robust-presets-result",
        id: worker.posted[1]!.message.id,
        ok: true,
        presets: presetsJson(),
      });
      await expect(retry).resolves.toMatchObject({
        robustFullBenchmark: { enableDeblur: true },
      });
    });

    it("rejects with ScanResultParseError when the presets shape drifted", async () => {
      const worker = new FakeWorker();
      const client = new ScannerClient(worker);

      const call = client.robustPresets();
      worker.emit({
        type: "robust-presets-result",
        id: worker.posted[0]!.message.id,
        ok: true,
        presets: { baseline: allOffConfig() }, // missing robustFast/robustFullBenchmark
      });
      await expect(call).rejects.toBeInstanceOf(ScanResultParseError);
    });
  });
});
