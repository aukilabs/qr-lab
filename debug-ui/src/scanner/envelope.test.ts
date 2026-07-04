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

  // Plan 4 Task 6: near_00 (a v1 fixture) now carries a populated
  // `codes`/`attempts`/`sample_regions`/`bits`, but an EMPTY `alignment` —
  // v1 has no alignment patterns at all (ISO 18004), so this is the
  // correct shape for this fixture, not a gap. A v7+ fixture (non-empty
  // `alignment`) is deferred to the Task 7 QA pass (real-browser
  // screenshot checks against a higher-version fixture) rather than
  // duplicated here — see the Rust-side `decode_trace_gate.rs` test's own
  // note.
  it("accepts the new Task 6 decode-trace fields on near_00", () => {
    const parsed = parseScanResult(loadSnapshot());

    expect(parsed.detections.codes).toHaveLength(1);
    expect(parsed.detections.codes[0]?.payload).toBe("Q:near_00:0");
    expect(parsed.detections.codes[0]?.version).toBe(1);
    expect(parsed.detections.codes[0]?.mirrored).toBe(false);

    expect(parsed.trace?.attempts.length).toBeGreaterThan(0);
    expect(parsed.trace?.attempts.some((a) => a.outcome === "decoded")).toBe(true);
    expect(parsed.trace?.attempts.every((a) => a.rounds.length > 0)).toBe(true);

    expect(parsed.trace?.alignment).toEqual([]);

    expect(parsed.trace?.sample_regions.length).toBeGreaterThan(0);
    expect(parsed.trace?.sample_regions[0]?.quad).toHaveLength(4);

    expect(parsed.trace?.bits).not.toBeNull();
    expect(parsed.trace?.bits?.dim).toBe(21);
    expect(parsed.trace?.bits?.words.length).toBeGreaterThan(0);
  });

  it("round-trips every field the snapshot carries", () => {
    const raw = loadSnapshot() as any;
    const parsed = parseScanResult(raw);

    expect(parsed.detections.timings).toEqual(raw.detections.timings);
    expect(parsed.detections.finders).toEqual(raw.detections.finders);
    expect(parsed.detections.triplets).toEqual(raw.detections.triplets);
    expect(parsed.detections.codes).toEqual(raw.detections.codes);
    expect(parsed.trace?.tiles).toEqual(raw.trace.tiles);
    expect(parsed.trace?.finders).toEqual(raw.trace.finders);
    expect(parsed.trace?.triplets).toEqual(raw.trace.triplets);
    expect(parsed.trace?.attempts).toEqual(raw.trace.attempts);
    expect(parsed.trace?.alignment).toEqual(raw.trace.alignment);
    expect(parsed.trace?.sample_regions).toEqual(raw.trace.sample_regions);
    expect(parsed.trace?.bits).toEqual(raw.trace.bits);
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

  // --- Plan 4 Task 6: mutation cases for the new decode-trace shapes ---

  it("throws with a path when detections.codes is missing", () => {
    const raw = loadSnapshot() as any;
    delete raw.detections.codes;
    expect(() => parseScanResult(raw)).toThrowError(/detections\.codes/);
  });

  it("throws with a path when a decoded code field has the wrong type", () => {
    const raw = loadSnapshot() as any;
    raw.detections.codes[0].version = "one";
    expect(() => parseScanResult(raw)).toThrowError(
      /detections\.codes\[0\]\.version/,
    );
  });

  it("throws with a path when a decoded code's corners quad is short", () => {
    const raw = loadSnapshot() as any;
    raw.detections.codes[0].corners = raw.detections.codes[0].corners.slice(0, 3);
    expect(() => parseScanResult(raw)).toThrowError(/detections\.codes\[0\]\.corners/);
  });

  it("throws with a path when a StageTimings field is missing (the 3 new Task 5/6 fields)", () => {
    for (const field of ["version_ns", "alignment_ns", "sample_decode_ns"]) {
      const raw = loadSnapshot() as any;
      delete raw.detections.timings[field];
      expect(() => parseScanResult(raw)).toThrowError(
        new RegExp(`detections\\.timings\\.${field}`),
      );
    }
  });

  it("throws with a path when trace.attempts is missing", () => {
    const raw = loadSnapshot() as any;
    delete raw.trace.attempts;
    expect(() => parseScanResult(raw)).toThrowError(/trace\.attempts/);
  });

  it("throws with a path when an attempt's rounds field has the wrong element type", () => {
    const raw = loadSnapshot() as any;
    raw.trace.attempts[0].rounds = [42];
    expect(() => parseScanResult(raw)).toThrowError(/trace\.attempts\[0\]\.rounds\[0\]/);
  });

  it("throws with a path when an attempt's optional timing_check has the wrong (non-null) type", () => {
    const raw = loadSnapshot() as any;
    raw.trace.attempts[0].timing_check = "21";
    expect(() => parseScanResult(raw)).toThrowError(/trace\.attempts\[0\]\.timing_check/);
  });

  it("accepts an attempt's optional version_bits: null", () => {
    const raw = loadSnapshot() as any;
    raw.trace.attempts[0].version_bits = null;
    const parsed = parseScanResult(raw);
    expect(parsed.trace?.attempts[0]?.version_bits).toBeNull();
  });

  it("throws with a path when trace.sample_regions is missing", () => {
    const raw = loadSnapshot() as any;
    delete raw.trace.sample_regions;
    expect(() => parseScanResult(raw)).toThrowError(/trace\.sample_regions/);
  });

  it("throws with a path when a sample region's module_rect is short", () => {
    const raw = loadSnapshot() as any;
    raw.trace.sample_regions[0].module_rect = [0, 0, 21];
    expect(() => parseScanResult(raw)).toThrowError(
      /trace\.sample_regions\[0\]\.module_rect/,
    );
  });

  it("accepts trace.bits: null (no candidate decoded this frame)", () => {
    const raw = loadSnapshot() as any;
    raw.trace.bits = null;
    const parsed = parseScanResult(raw);
    expect(parsed.trace?.bits).toBeNull();
  });

  it("throws with a path when trace.bits.words has the wrong element type", () => {
    const raw = loadSnapshot() as any;
    raw.trace.bits.words = ["not-a-number"];
    expect(() => parseScanResult(raw)).toThrowError(/trace\.bits\.words\[0\]/);
  });

  it("throws with a path when trace.alignment has a malformed entry", () => {
    const raw = loadSnapshot() as any;
    // near_00's own `alignment` is legitimately empty (v1) — inject a
    // synthetic entry to exercise `AlignmentTraceEntry` parsing itself.
    raw.trace.alignment = [{ predicted: [1, 2], found: "nope" }];
    expect(() => parseScanResult(raw)).toThrowError(/trace\.alignment\[0\]\.found/);
  });

  it("accepts a well-formed synthetic trace.alignment entry with found: null", () => {
    const raw = loadSnapshot() as any;
    raw.trace.alignment = [{ predicted: [1.5, 2.5], found: null }];
    const parsed = parseScanResult(raw);
    expect(parsed.trace?.alignment).toEqual([{ predicted: [1.5, 2.5], found: null }]);
  });
});
