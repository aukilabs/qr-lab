import { describe, expect, it, vi } from "vitest";
import { ScannerClient, StaleScanError, type ScannerWorkerLike } from "./client";
import { ScanResultParseError } from "./types";

/** Minimal in-memory stand-in for the real Worker, driven manually by
 * tests: `emit()` synchronously dispatches a fake `MessageEvent` to
 * whatever listener ScannerClient registered, so tests control exactly
 * when (and in what order) "worker" responses arrive. */
class FakeWorker implements ScannerWorkerLike {
  posted: Array<{ message: any; transfer?: Transferable[] }> = [];
  terminated = false;
  private listeners: Array<(ev: MessageEvent) => void> = [];

  postMessage(message: unknown, transfer?: Transferable[]): void {
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
