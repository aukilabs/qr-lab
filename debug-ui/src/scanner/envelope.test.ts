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

  // Plan 5 Task 3: the committed snapshot was regenerated with `refine:
  // true` specifically so this shape is populated (not just `null`) — see
  // `envelope_snapshot.rs`'s own `REFINE` constant doc.
  it("accepts the new Plan 5 Task 3 refined-corners fields on near_00", () => {
    const parsed = parseScanResult(loadSnapshot());

    expect(parsed.detections.codes[0]?.refined_corners).not.toBeNull();
    expect(parsed.detections.codes[0]?.refined_corners).toHaveLength(4);

    expect(parsed.trace?.refine).not.toBeNull();
    expect(parsed.trace?.refine?.edges).toHaveLength(4);
    expect(parsed.trace?.refine?.corner_refined).toHaveLength(4);
    for (const edge of parsed.trace?.refine?.edges ?? []) {
      expect(edge.points_probed).toBeGreaterThanOrEqual(0);
    }
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
    expect(parsed.trace?.refine).toEqual(raw.trace.refine);
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

  it("throws with a path when a StageTimings field is missing (the 3 new Task 5/6 fields, plus Plan 5 Task 3's refine_ns)", () => {
    for (const field of ["version_ns", "alignment_ns", "sample_decode_ns", "refine_ns"]) {
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

  // QA regression (Plan 4 Task 7): the committed snapshot is JSON text, so
  // every `Option::None` in it is `null`. The LIVE wasm binding
  // (`serde_wasm_bindgen::to_value`) instead renders `None` as `undefined`
  // (key present, value `undefined`) — a real live-browser scan of any
  // fixture below version 7 hit exactly this on `version_bits` before the
  // "OrNull" parsers below were widened to treat `undefined` the same as
  // `null`. See the doc comment on `parseNumberOrNull`.
  it("accepts an attempt's optional version_bits: undefined (the real wasm-binding shape)", () => {
    const raw = loadSnapshot() as any;
    raw.trace.attempts[0].version_bits = undefined;
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

  // QA regression (Plan 4 Task 7) — see the `version_bits: undefined` test
  // above for why `undefined` (not just `null`) must be accepted here too.
  it("accepts trace.bits: undefined (the real wasm-binding shape)", () => {
    const raw = loadSnapshot() as any;
    raw.trace.bits = undefined;
    const parsed = parseScanResult(raw);
    expect(parsed.trace?.bits).toBeNull();
  });

  // Plan 5 Task 3 — same "undefined must parse like null" wasm-binding
  // regression as `bits`/`version_bits` above, for the new refine fields.
  it("accepts a decoded code's refined_corners: null and trace.refine: undefined", () => {
    const raw = loadSnapshot() as any;
    raw.detections.codes[0].refined_corners = null;
    raw.trace.refine = undefined;
    const parsed = parseScanResult(raw);
    expect(parsed.detections.codes[0]?.refined_corners).toBeNull();
    expect(parsed.trace?.refine).toBeNull();
  });

  it("throws with a path when trace.bits.words has the wrong element type", () => {
    const raw = loadSnapshot() as any;
    // 21 entries so the length invariant (checked after element types)
    // still holds — this case must fail on the ELEMENT type specifically.
    raw.trace.bits.words = ["not-a-number", ...new Array(20).fill(0)];
    expect(() => parseScanResult(raw)).toThrowError(/trace\.bits\.words\[0\]/);
  });

  it("throws with a path when trace.bits.words violates the dim*ceil(dim/32) packing invariant", () => {
    // Truncated words array (one word short of near_00's 21).
    const raw = loadSnapshot() as any;
    raw.trace.bits.words = raw.trace.bits.words.slice(0, -1);
    expect(() => parseScanResult(raw)).toThrowError(/trace\.bits\.words.*21 packed words/);

    // Padded words array (one extra word).
    const raw2 = loadSnapshot() as any;
    raw2.trace.bits.words = [...raw2.trace.bits.words, 0];
    expect(() => parseScanResult(raw2)).toThrowError(/trace\.bits\.words.*21 packed words/);

    // Consistency check: a dim crossing the 32-bit word boundary (v10:
    // dim 57 -> 2 words/row -> 114 words) parses fine — the invariant is
    // dim * ceil(dim/32), not dim itself.
    const raw3 = loadSnapshot() as any;
    raw3.trace.bits = { dim: 57, words: new Array(114).fill(0) };
    expect(parseScanResult(raw3).trace?.bits?.words).toHaveLength(114);
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

  // QA regression (Plan 4 Task 7) — see the `version_bits: undefined` test
  // above; `found: undefined` is the real wasm-binding shape for a
  // not-located alignment slot.
  it("accepts a synthetic trace.alignment entry with found: undefined (the real wasm-binding shape)", () => {
    const raw = loadSnapshot() as any;
    raw.trace.alignment = [{ predicted: [1.5, 2.5], found: undefined }];
    const parsed = parseScanResult(raw);
    expect(parsed.trace?.alignment).toEqual([{ predicted: [1.5, 2.5], found: null }]);
  });
});
