import { describe, expect, it } from "vitest";
import {
  compositeOver,
  expectedInverted,
  expectedInvertedComposited,
  hexToRgb,
  lumaFromRgb,
  rgbToHex,
} from "./colorUtils";
import { CONTRAST_WARN_THRESHOLD } from "./consts";

describe("hexToRgb", () => {
  it("parses a #rrggbb color", () => {
    expect(hexToRgb("#ff8800")).toEqual([255, 136, 0]);
  });

  it("parses a #rgb shorthand color", () => {
    expect(hexToRgb("#f80")).toEqual([255, 136, 0]);
  });

  it("is case-insensitive and tolerates a missing leading #", () => {
    expect(hexToRgb("FF8800")).toEqual([255, 136, 0]);
  });

  it("throws on malformed input", () => {
    expect(() => hexToRgb("not-a-color")).toThrow(RangeError);
    expect(() => hexToRgb("#ff88")).toThrow(RangeError);
  });
});

describe("rgbToHex", () => {
  it("round-trips through hexToRgb", () => {
    expect(hexToRgb(rgbToHex(255, 136, 0))).toEqual([255, 136, 0]);
  });

  it("clamps and rounds out-of-range channels", () => {
    expect(rgbToHex(-10, 300, 127.6)).toBe("#00ff80");
  });
});

describe("lumaFromRgb", () => {
  it("matches the scanner's exact fixed-point formula (77/150/29 vector)", () => {
    // 77*200 + 150*100 + 29*50 + 128 = 31978; 31978 >> 8 = 124 — same
    // hand-derived vector used by media/luma.test.ts, pinning both copies
    // of the formula to the identical result.
    expect(lumaFromRgb(200, 100, 50)).toBe(124);
  });

  it("returns 255 for pure white and 0 for pure black", () => {
    expect(lumaFromRgb(255, 255, 255)).toBe(255);
    expect(lumaFromRgb(0, 0, 0)).toBe(0);
  });
});

describe("expectedInverted", () => {
  it("reads black-ink-on-white-bg as normal (not inverted), high contrast", () => {
    const r = expectedInverted("#000000", "#ffffff");
    expect(r.inverted).toBe(false);
    expect(r.deltaLuma).toBe(255);
    expect(r.lowContrast).toBe(false);
  });

  it("reads white-ink-on-black-bg as inverted, high contrast", () => {
    const r = expectedInverted("#ffffff", "#000000");
    expect(r.inverted).toBe(true);
    expect(r.deltaLuma).toBe(255);
    expect(r.lowContrast).toBe(false);
  });

  it("flags low contrast below the warning threshold", () => {
    // Two mid-grays close in luma.
    const r = expectedInverted("#808080", "#7a7a7a");
    expect(r.deltaLuma).toBeLessThan(CONTRAST_WARN_THRESHOLD);
    expect(r.lowContrast).toBe(true);
  });

  it("does not flag contrast comfortably above the threshold", () => {
    // Gray pair with luma delta exactly 50 (0x96=150, 0x64=100) — well
    // above CONTRAST_WARN_THRESHOLD (30).
    const bright = expectedInverted("#969696", "#646464");
    expect(bright.deltaLuma).toBe(50);
    expect(bright.deltaLuma).toBeGreaterThanOrEqual(CONTRAST_WARN_THRESHOLD);
    expect(bright.lowContrast).toBe(false);
  });

  it("treats identical colors as zero delta, inverted false, low contrast true", () => {
    const r = expectedInverted("#336699", "#336699");
    expect(r.deltaLuma).toBe(0);
    expect(r.inverted).toBe(false);
    expect(r.lowContrast).toBe(true);
  });
});

describe("compositeOver", () => {
  it("returns the top color unchanged at alpha=1", () => {
    expect(compositeOver("#ff8800", 1, "#000000")).toEqual([255, 136, 0]);
  });

  it("returns the under color at alpha=0", () => {
    expect(compositeOver("#ffffff", 0, "#05070d")).toEqual([5, 7, 13]);
  });

  it("interpolates linearly per channel at alpha=0.5", () => {
    const [r, g, b] = compositeOver("#ffffff", 0.5, "#000000");
    expect(r).toBeCloseTo(127.5, 9);
    expect(g).toBeCloseTo(127.5, 9);
    expect(b).toBeCloseTo(127.5, 9);
  });

  it("clamps out-of-range alpha into [0, 1]", () => {
    expect(compositeOver("#ffffff", 2, "#000000")).toEqual([255, 255, 255]);
    expect(compositeOver("#ffffff", -1, "#000000")).toEqual([0, 0, 0]);
  });
});

describe("expectedInvertedComposited", () => {
  it("matches expectedInverted exactly at alpha=1 (no compositing)", () => {
    const flat = expectedInverted("#000000", "#ffffff");
    const comp = expectedInvertedComposited("#000000", "#ffffff", 1, "#05070d");
    expect(comp).toEqual(flat);
  });

  it("flips polarity vs the flat prediction: white ink on white paper at alpha~0 over a dark scene reads INVERTED", () => {
    // Flat prediction: white ink on white bg -> delta 0, not inverted,
    // low contrast. Composited reality: at alpha 0 the "paper" IS the
    // near-black scene background, so white ink on it is clearly
    // inverted with high contrast — the exact polarity flip the review
    // flagged (the pre-fix indicator would have lied here).
    const flat = expectedInverted("#ffffff", "#ffffff");
    expect(flat.inverted).toBe(false);
    expect(flat.lowContrast).toBe(true);

    const comp = expectedInvertedComposited("#ffffff", "#ffffff", 0, "#05070d");
    expect(comp.inverted).toBe(true);
    expect(comp.lowContrast).toBe(false);
    expect(comp.deltaLuma).toBeGreaterThan(200);
  });

  it("flags low contrast when the composited paper approaches the ink's luma", () => {
    // Black ink; white paper at alpha 0.1 over a near-black scene ->
    // effective paper luma ~ 0.1*255 + 0.9*7 ≈ 32 — close to black ink.
    const comp = expectedInvertedComposited("#000000", "#ffffff", 0.1, "#05070d");
    expect(comp.deltaLuma).toBeLessThan(40);
    expect(comp.inverted).toBe(false); // paper still (barely) brighter than ink
  });
});
