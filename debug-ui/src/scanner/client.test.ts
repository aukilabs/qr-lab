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
      timings: { tiles_ns: 0, finders_ns: 0, triplets_ns: 0 },
    },
    trace: null,
  };
}

function makeRgba(len: number): Uint8ClampedArray {
  return new Uint8ClampedArray(len);
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
});
