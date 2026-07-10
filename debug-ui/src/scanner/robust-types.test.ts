import { describe, expect, it } from "vitest";
import snapshot from "./__snapshots__/envelope.robust.shadow_04.json";
import {
  BARE_VARIANT_KINDS,
  parseRobustPresets,
  parseRobustScanResult,
  parseVariantKind,
  stageColor,
  variantKindLabel,
  type RobustConfig,
  type VariantKind,
} from "./robust-types";
import { ScanResultParseError } from "./types";

// `structuredClone` so each test can mutate its own copy without touching
// the module-level import (which vitest/vite otherwise shares across every
// test in this file) or the committed fixture on disk.
function loadSnapshot(): unknown {
  return structuredClone(snapshot);
}

function robustConfig(overrides: Partial<RobustConfig> = {}): RobustConfig {
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
    ...overrides,
  };
}


describe("parseRobustScanResult", () => {
  it("accepts the real Rust-generated robust envelope snapshot", () => {
    const parsed = parseRobustScanResult(loadSnapshot());

    expect(parsed.robust.codes).toHaveLength(1);
    expect(parsed.robust.codes[0]?.variant).toBe("SauvolaThreshold");
    expect(parsed.robust.codes[0]?.stage).toBe(3);
    expect(parsed.robust.codes[0]?.code.payload).toBe("Q:shadow_04:0");
    expect(parsed.robust.codes[0]?.corners_source).toHaveLength(4);
    expect(parsed.robust.codes[0]?.refined_corners_source).toHaveLength(4);

    expect(parsed.robust.variants.length).toBeGreaterThan(1);
    expect(parsed.robust.variants[0]?.kind).toBe("Baseline");
    expect(parsed.robust.variants[0]?.stage).toBe(0);
    expect(parsed.robust.variants.some((v) => v.new_codes > 0)).toBe(true);

    expect(parsed.robust.early_exited).toBe(false);
    expect(parsed.robust.budget_exhausted).toBe(false);
    expect(parsed.robust.triplet_evidence).toHaveLength(1);

    // The unified detections ride every robust envelope (one-pipeline
    // contract) — the ladder-recovered code appears there like any classic
    // decode.
    expect(parsed.detections.codes).toHaveLength(1);
    expect(parsed.detections.codes[0]?.payload).toBe("Q:shadow_04:0");
    expect(parsed.detections.finders.length).toBeGreaterThan(0);
    expect(parsed.detections.triplets.length).toBeGreaterThan(0);
    // The committed snapshot was generated without capture — only the
    // filmstrip is capture-gated.
    expect(parsed.snapshots).toBeNull();
    expect(parsed.scan_width).toBe(1280);
    expect(parsed.scan_height).toBe(720);
  });

  it("round-trips the tagged VariantKind shapes the snapshot carries", () => {
    const parsed = parseRobustScanResult(loadSnapshot());
    const kinds = parsed.robust.variants.map((v) => v.kind);
    expect(kinds).toContainEqual({ Pyramid: { level: 1 } });
    expect(kinds).toContainEqual({ ThresholdOffset: { offset: -8 } });
    expect(kinds).toContainEqual({ ThresholdOffset: { offset: 8 } });
  });

  // The serde-wasm-bindgen contract (see the module doc, and types.ts's
  // parseNumberOrNull): the JSON snapshot renders Option::None as `null`,
  // the LIVE binding renders it as `undefined` — BOTH must parse.
  it("accepts snapshots: null (the JSON-snapshot shape)", () => {
    const raw = loadSnapshot() as any;
    raw.snapshots = null;
    const parsed = parseRobustScanResult(raw);
    expect(parsed.snapshots).toBeNull();
  });

  it("accepts snapshots: undefined (the real wasm-binding shape)", () => {
    const raw = loadSnapshot() as any;
    raw.snapshots = undefined;
    const parsed = parseRobustScanResult(raw);
    expect(parsed.snapshots).toBeNull();
  });

  it("accepts refined_corners_source: null AND undefined on a robust code", () => {
    for (const missing of [null, undefined]) {
      const raw = loadSnapshot() as any;
      raw.robust.codes[0].refined_corners_source = missing;
      const parsed = parseRobustScanResult(raw);
      expect(parsed.robust.codes[0]?.refined_corners_source).toBeNull();
    }
  });

  // Capture-on shape, built by hand: over the real wasm boundary `luma`
  // arrives as a Uint8Array (serde_bytes) — the parser must accept it AND
  // the number[] a JSON snapshot would carry, normalizing both.
  it("parses a capture-on result with Uint8Array luma (the real wasm-binding shape)", () => {
    const raw = loadSnapshot() as any;
    raw.snapshots = [
      {
        kind: "Baseline",
        stage: 0,
        width: 2,
        height: 2,
        luma: new Uint8Array([0, 85, 170, 255]),
      },
      {
        kind: { Pyramid: { level: 2 } },
        stage: 1,
        width: 1,
        height: 2,
        luma: new Uint8Array([7, 9]),
      },
    ];

    const parsed = parseRobustScanResult(raw);
    expect(parsed.snapshots).toHaveLength(2);
    expect(parsed.snapshots?.[0]?.luma).toBeInstanceOf(Uint8Array);
    expect(Array.from(parsed.snapshots![0]!.luma)).toEqual([0, 85, 170, 255]);
    expect(parsed.snapshots?.[1]?.kind).toEqual({ Pyramid: { level: 2 } });
  });

  it("parses a capture-on result with number[] luma (the JSON-snapshot shape), normalizing to Uint8Array", () => {
    const raw = loadSnapshot() as any;
    raw.snapshots = [
      { kind: "ShadowNormalized", stage: 3, width: 2, height: 1, luma: [12, 34] },
    ];

    const parsed = parseRobustScanResult(raw);
    expect(parsed.snapshots?.[0]?.luma).toBeInstanceOf(Uint8Array);
    expect(Array.from(parsed.snapshots![0]!.luma)).toEqual([12, 34]);
  });

  it("throws with a path when a snapshot's luma length doesn't match width * height", () => {
    const raw = loadSnapshot() as any;
    raw.snapshots = [{ kind: "Baseline", stage: 0, width: 2, height: 2, luma: [1, 2, 3] }];
    expect(() => parseRobustScanResult(raw)).toThrowError(/snapshots\[0\]\.luma.*4 luma bytes/);
  });

  it("throws with a path when robust.codes is missing", () => {
    const raw = loadSnapshot() as any;
    delete raw.robust.codes;
    expect(() => parseRobustScanResult(raw)).toThrowError(/robust\.codes/);
  });

  it("throws with a path when a variant record field has the wrong type", () => {
    const raw = loadSnapshot() as any;
    raw.robust.variants[0].new_codes = "0";
    expect(() => parseRobustScanResult(raw)).toThrowError(/robust\.variants\[0\]\.new_codes/);
  });

  it("throws with a path when a robust code's corners_source quad is short", () => {
    const raw = loadSnapshot() as any;
    raw.robust.codes[0].corners_source = raw.robust.codes[0].corners_source.slice(0, 3);
    expect(() => parseRobustScanResult(raw)).toThrowError(/robust\.codes\[0\]\.corners_source/);
  });

  it("throws with a path when triplet_evidence has a malformed point", () => {
    const raw = loadSnapshot() as any;
    raw.robust.triplet_evidence = [[1]];
    expect(() => parseRobustScanResult(raw)).toThrowError(/robust\.triplet_evidence\[0\]/);
  });

  it("throws ScanResultParseError when the top-level value isn't an object", () => {
    expect(() => parseRobustScanResult(null)).toThrowError(ScanResultParseError);
    expect(() => parseRobustScanResult("nope")).toThrowError(ScanResultParseError);
  });
});

describe("parseVariantKind", () => {
  it("accepts every bare (unit) kind", () => {
    for (const kind of BARE_VARIANT_KINDS) {
      expect(parseVariantKind(kind, "k")).toBe(kind);
    }
  });

  it("accepts every tagged (payload) kind shape", () => {
    expect(parseVariantKind({ Pyramid: { level: 1 } }, "k")).toEqual({ Pyramid: { level: 1 } });
    expect(parseVariantKind({ ThresholdOffset: { offset: -8 } }, "k")).toEqual({
      ThresholdOffset: { offset: -8 },
    });
    expect(parseVariantKind({ UpscaledRoi: { factor: 3 } }, "k")).toEqual({
      UpscaledRoi: { factor: 3 },
    });
    expect(parseVariantKind({ DirectionalSharpened: { theta_deg: 135 } }, "k")).toEqual({
      DirectionalSharpened: { theta_deg: 135 },
    });
    expect(parseVariantKind({ VanCittert: { theta_deg: 45, len: 15 } }, "k")).toEqual({
      VanCittert: { theta_deg: 45, len: 15 },
    });
  });

  it("throws on an unknown bare kind, unknown tag, or multi-key object", () => {
    expect(() => parseVariantKind("Blurred", "k")).toThrowError(/unknown VariantKind "Blurred"/);
    expect(() => parseVariantKind({ Mystery: { x: 1 } }, "k")).toThrowError(
      /unknown VariantKind tag "Mystery"/,
    );
    expect(() =>
      parseVariantKind({ Pyramid: { level: 1 }, ThresholdOffset: { offset: 8 } }, "k"),
    ).toThrowError(/single-key/);
  });

  it("throws with a path when a payload field is missing or mistyped", () => {
    expect(() => parseVariantKind({ Pyramid: {} }, "k")).toThrowError(/k\.Pyramid\.level/);
    expect(() => parseVariantKind({ VanCittert: { theta_deg: 45, len: "15" } }, "k")).toThrowError(
      /k\.VanCittert\.len/,
    );
  });
});

describe("variantKindLabel", () => {
  // Every VariantKind shape, against the labels the Plan 6 spec pins.
  const cases: Array<[VariantKind, string]> = [
    ["Baseline", "baseline"],
    [{ Pyramid: { level: 1 } }, "pyramid 0.5×"],
    [{ Pyramid: { level: 2 } }, "pyramid 0.25×"],
    [{ ThresholdOffset: { offset: -8 } }, "threshold −8"],
    [{ ThresholdOffset: { offset: 8 } }, "threshold +8"],
    ["LowContrastFloor", "low contrast floor"],
    ["SauvolaThreshold", "Sauvola threshold"],
    ["ShadowNormalized", "background divide"],
    ["Sharpened", "unsharp"],
    ["Upscaled2x", "2× upscale"],
    ["Upscaled3x", "3× upscale"],
    [{ UpscaledRoi: { factor: 2 } }, "ROI 2×"],
    [{ UpscaledRoi: { factor: 3 } }, "ROI 3×"],
    [{ DirectionalSharpened: { theta_deg: 45 } }, "directional 45°"],
    [{ VanCittert: { theta_deg: 45, len: 15 } }, "Van Cittert 45°·15px"],
  ];

  it.each(cases)("labels %j as %s", (kind, label) => {
    expect(variantKindLabel(kind)).toBe(label);
  });
});

describe("stageColor", () => {
  it("returns 7 distinct colors across stages 0-6", () => {
    const colors = [0, 1, 2, 3, 4, 5, 6].map(stageColor);
    expect(new Set(colors).size).toBe(7);
  });

  it("stage 0 is the decoded-layer green family", () => {
    expect(stageColor(0)).toBe("#00e676");
  });

  it("clamps out-of-range stages instead of returning undefined", () => {
    expect(stageColor(-1)).toBe(stageColor(0));
    // Stage 7 (cross-variant pool) is the top real stage; clamp lands there.
    expect(stageColor(99)).toBe(stageColor(7));
    expect(stageColor(7)).not.toBe(stageColor(6));
  });
});

describe("parseRobustPresets", () => {
  it("round-trips a well-formed presets object", () => {
    const presets = {
      baseline: robustConfig(),
      robustFast: robustConfig({
        enableMultiScale: true,
        enableEarlyExit: true,
        maxVariantsPerFrame: 4,
      }),
      robustFullBenchmark: robustConfig({
        enableMultiScale: true,
        enableContrastNormalization: true,
        enableShadowNormalization: true,
        enableAdaptiveThresholding: true,
        enableSharpening: true,
        enableDeblur: true,
        enableLowResUpscaling: true,
      }),
    };
    expect(parseRobustPresets(structuredClone(presets))).toEqual(presets);
  });

  it("throws with a path on a missing preset or flag", () => {
    expect(() => parseRobustPresets({ baseline: robustConfig() })).toThrowError(/robustFast/);
    const broken: Record<string, unknown> = {
      baseline: robustConfig(),
      robustFast: robustConfig(),
      robustFullBenchmark: { ...robustConfig(), enableDeblur: undefined },
    };
    delete (broken.robustFullBenchmark as Record<string, unknown>).enableDeblur;
    expect(() => parseRobustPresets(broken)).toThrowError(/robustFullBenchmark\.enableDeblur/);
  });
});
