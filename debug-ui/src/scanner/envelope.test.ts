import { describe, expect, it } from "vitest";
import snapshot from "./__snapshots__/envelope.near_00.json";
import { parseScanResult } from "./types";

// `structuredClone` so each test can mutate its own copy without touching
// the module-level import (which vitest/vite otherwise shares across every
// test in this file) or the committed fixture on disk.
function loadSnapshot(): unknown {
  return structuredClone(snapshot);
}

describe("parseScanResult", () => {
  it("accepts the real Rust-generated envelope snapshot", () => {
    const parsed = parseScanResult(loadSnapshot());
    expect(parsed.detections.finders.length).toBeGreaterThan(0);
    expect(parsed.detections.triplets.length).toBeGreaterThan(0);
    expect(parsed.trace).not.toBeNull();
    expect(parsed.trace?.tiles).not.toBeNull();
  });

  it("round-trips every field the snapshot carries", () => {
    const raw = loadSnapshot() as any;
    const parsed = parseScanResult(raw);

    expect(parsed.detections.timings).toEqual(raw.detections.timings);
    expect(parsed.detections.finders).toEqual(raw.detections.finders);
    expect(parsed.detections.triplets).toEqual(raw.detections.triplets);
    expect(parsed.trace?.tiles).toEqual(raw.trace.tiles);
    expect(parsed.trace?.finders).toEqual(raw.trace.finders);
    expect(parsed.trace?.triplets).toEqual(raw.trace.triplets);
  });

  it("throws with a path when a required array is missing", () => {
    const raw = loadSnapshot() as any;
    delete raw.detections.finders;
    expect(() => parseScanResult(raw)).toThrowError(/detections\.finders/);
  });

  it("throws with a path when a nested field has the wrong type", () => {
    const raw = loadSnapshot() as any;
    raw.trace.tiles.thresholds = "not an array";
    expect(() => parseScanResult(raw)).toThrowError(/trace\.tiles\.thresholds/);
  });

  it("throws with a path when a finder candidate field is the wrong type", () => {
    const raw = loadSnapshot() as any;
    raw.detections.finders[0].inverted = "yes";
    expect(() => parseScanResult(raw)).toThrowError(
      /detections\.finders\[0\]\.inverted/,
    );
  });

  it("throws when the top-level value isn't an object", () => {
    expect(() => parseScanResult(null)).toThrowError(/ScanResult/);
    expect(() => parseScanResult("nope")).toThrowError(/ScanResult/);
  });

  it("accepts trace: null (no-trace scan_rgba call)", () => {
    const raw = loadSnapshot() as any;
    raw.trace = null;
    const parsed = parseScanResult(raw);
    expect(parsed.trace).toBeNull();
  });

  it("accepts tiles: null within a populated trace", () => {
    const raw = loadSnapshot() as any;
    raw.trace.tiles = null;
    const parsed = parseScanResult(raw);
    expect(parsed.trace?.tiles).toBeNull();
  });

  it("throws with a path when a renamed/typo'd field is missing", () => {
    const raw = loadSnapshot() as any;
    raw.detections.finders[0].modul = raw.detections.finders[0].module;
    delete raw.detections.finders[0].module;
    expect(() => parseScanResult(raw)).toThrowError(
      /detections\.finders\[0\]\.module/,
    );
  });
});
