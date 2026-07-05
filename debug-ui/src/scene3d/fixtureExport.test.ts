import { describe, expect, it } from "vitest";
import { PerspectiveCamera, Vector3 } from "three";
import {
  buildFixtureMeta,
  defaultFixtureName,
  eccLetterFromIndex,
  moduleSizeFromCorners,
  opaquePlateFromAlpha,
  probeInvertedFromRgba,
  slugifyPayload,
  versionFromDim,
  type Point2,
} from "./fixtureExport";
import { moduleRegionLocalCornersArray } from "./moduleRegion";
import { projectAllToPixels } from "./projection";
import { QUIET_MODULES } from "./consts";
import { parseGroundTruth } from "../overlays/groundtruth-types";

describe("eccLetterFromIndex", () => {
  it("maps 0..3 to l/m/q/h", () => {
    expect(eccLetterFromIndex(0)).toBe("l");
    expect(eccLetterFromIndex(1)).toBe("m");
    expect(eccLetterFromIndex(2)).toBe("q");
    expect(eccLetterFromIndex(3)).toBe("h");
  });

  it("rejects an out-of-range index", () => {
    expect(() => eccLetterFromIndex(4)).toThrow(RangeError);
    expect(() => eccLetterFromIndex(-1)).toThrow(RangeError);
  });
});

describe("versionFromDim", () => {
  it("derives version from dim = 4*version + 17", () => {
    expect(versionFromDim(21)).toBe(1);
    expect(versionFromDim(25)).toBe(2);
    expect(versionFromDim(177)).toBe(40);
  });

  it("rejects a dim that doesn't fit the 4v+17 formula", () => {
    expect(() => versionFromDim(22)).toThrow(RangeError);
  });

  it("rejects a dim below the smallest valid version", () => {
    expect(() => versionFromDim(17)).toThrow(RangeError);
  });
});

describe("slugifyPayload / defaultFixtureName", () => {
  it("lowercases and collapses non-alphanumeric runs to a single underscore", () => {
    expect(slugifyPayload("HTTPS://AUKILABS.COM/CPUSCANNER2/SCENE3D")).toBe(
      "https_aukilabs_com_cpuscanner2_scene3d",
    );
  });

  it("trims leading/trailing underscores", () => {
    expect(slugifyPayload("!!!hello!!!")).toBe("hello");
  });

  it("falls back to 'payload' for an all-punctuation string", () => {
    expect(slugifyPayload("!!!")).toBe("payload");
  });

  it("prefixes the slug with scene_", () => {
    expect(defaultFixtureName("HELLO")).toBe("scene_hello");
  });
});

describe("opaquePlateFromAlpha", () => {
  it("is true at alpha=1 and false below it", () => {
    expect(opaquePlateFromAlpha(1)).toBe(true);
    expect(opaquePlateFromAlpha(0.99)).toBe(false);
    expect(opaquePlateFromAlpha(0)).toBe(false);
  });
});

describe("buildFixtureMeta", () => {
  const meta = buildFixtureMeta({
    name: "scene_test",
    width: 960,
    height: 960,
    camera: { fx: 830, fy: 830, cx: 479.5, cy: 479.5 },
    blurSigma: 1.5,
    noiseSigma: 2,
    exposureOffset: 10,
    code: {
      payload: "HELLO",
      version: 1,
      eccLetter: "m",
      physicalSizeM: 0.1,
      distanceM: 0.3,
      tiltDeg: 12.5,
      moduleSizePx: 20,
      cornersPx: [
        [10, 10],
        [110, 10],
        [110, 110],
        [10, 110],
      ],
      inverted: false,
      opaquePlate: true,
    },
  });

  it("round-trips its single code entry through parseGroundTruth", () => {
    expect(() => parseGroundTruth(meta.codes[0])).not.toThrow();
    const parsed = parseGroundTruth(meta.codes[0]);
    expect(parsed).toEqual({
      corners_px: [
        [10, 10],
        [110, 10],
        [110, 110],
        [10, 110],
      ],
      version: 1,
      module_size_px: 20,
      inverted: false,
      opaque_plate: true,
      payload: "HELLO",
    });
  });

  it("puts exposure_offset as a top-level extra field (not in the generator schema)", () => {
    expect(meta.exposure_offset).toBe(10);
  });

  it("uses a fixed seed of 0", () => {
    expect(meta.seed).toBe(0);
  });

  it("sets tilt_azimuth_deg/inplane_deg to 0 (not losslessly recoverable)", () => {
    expect(meta.codes[0]!.tilt_azimuth_deg).toBe(0);
    expect(meta.codes[0]!.inplane_deg).toBe(0);
  });

  it("sets mirrored to false", () => {
    expect(meta.codes[0]!.mirrored).toBe(false);
  });

  it("mirrors width/height/camera/blur/noise inputs verbatim", () => {
    expect(meta.width).toBe(960);
    expect(meta.height).toBe(960);
    expect(meta.camera).toEqual({ fx: 830, fy: 830, cx: 479.5, cy: 479.5 });
    expect(meta.blur_sigma).toBe(1.5);
    expect(meta.noise_sigma).toBe(2);
  });

  it("carries the ecc letter and physical_size_m/distance_m/tilt_deg through verbatim", () => {
    expect(meta.codes[0]!.ecc).toBe("m");
    expect(meta.codes[0]!.physical_size_m).toBe(0.1);
    expect(meta.codes[0]!.distance_m).toBe(0.3);
    expect(meta.codes[0]!.tilt_deg).toBe(12.5);
  });

  it("always produces exactly one code entry", () => {
    expect(meta.codes).toHaveLength(1);
  });
});

describe("moduleSizeFromCorners", () => {
  /** Replicate Scene3D's exact truth-projection path at a given camera
   * pose: module-region local corners -> world (identity plane transform)
   * -> projected readback px. */
  function projectedCorners(
    cameraPos: [number, number, number],
    dim: number,
    physicalSize: number,
    resolution: number,
  ): [Point2, Point2, Point2, Point2] {
    const cam = new PerspectiveCamera(50, 1, 0.01, 100);
    cam.position.set(...cameraPos);
    cam.lookAt(0, 0, 0);
    cam.updateMatrixWorld(true);
    const local = moduleRegionLocalCornersArray(dim, QUIET_MODULES, physicalSize);
    const world = local.map((c) => new Vector3(c[0], c[1], c[2]));
    const px = projectAllToPixels(world, cam, resolution, resolution);
    return [px[0]!, px[1]!, px[2]!, px[3]!] as [Point2, Point2, Point2, Point2];
  }

  it("equals |TR-TL|/dim exactly (generate.py's derivation)", () => {
    const corners: [Point2, Point2, Point2, Point2] = [
      [100, 100],
      [390, 110],
      [380, 400],
      [95, 390],
    ];
    const expected = Math.hypot(390 - 100, 110 - 100) / 29;
    expect(moduleSizeFromCorners(corners, 29)).toBeCloseTo(expected, 12);
  });

  it("tracks the projection at a NON-default pose and differs from the old pose-invariant constant", () => {
    // Off-axis camera pose (translated in x/y, dollied out) — the review
    // fix's mandated scenario. dim=29 (v3), physicalSize=0.15m,
    // resolution=960: the OLD export constant was 960/(29+8) ≈ 25.95
    // regardless of pose; the actual projected TL->TR edge at this pose
    // is far shorter per module.
    const dim = 29;
    const resolution = 960;
    const corners = projectedCorners([0.1, 0.08, 0.6], dim, 0.15, resolution);
    const derived = moduleSizeFromCorners(corners, dim);
    const [tl, tr] = corners;
    expect(derived).toBeCloseTo(Math.hypot(tr[0] - tl[0], tr[1] - tl[1]) / dim, 12);

    const oldConstant = resolution / (dim + 2 * QUIET_MODULES);
    expect(Math.abs(derived - oldConstant)).toBeGreaterThan(1); // clearly different, not rounding noise
  });

  it("shrinks as the camera moves away (pose-DEPENDENT, unlike the old constant)", () => {
    const dim = 29;
    const near = moduleSizeFromCorners(projectedCorners([0, 0, 0.3], dim, 0.15, 960), dim);
    const far = moduleSizeFromCorners(projectedCorners([0, 0, 0.9], dim, 0.15, 960), dim);
    expect(far).toBeLessThan(near);
    expect(near / far).toBeGreaterThan(2); // ~3x distance ratio -> ~3x size ratio
  });

  it("rejects an invalid dim", () => {
    const corners: [Point2, Point2, Point2, Point2] = [
      [0, 0],
      [29, 0],
      [29, 29],
      [0, 29],
    ];
    expect(() => moduleSizeFromCorners(corners, 0)).toThrow(RangeError);
  });
});

describe("probeInvertedFromRgba", () => {
  /** 64x64 rgba buffer: `outer` gray everywhere, `inner` gray inside the
   * square spanned by `corners` (axis-aligned for simplicity — the probe
   * only samples two points near the TL corner's diagonal). */
  function makeFrame(
    corners: [Point2, Point2, Point2, Point2],
    inner: number,
    outer: number,
  ): Uint8ClampedArray {
    const w = 64;
    const rgba = new Uint8ClampedArray(w * w * 4);
    const [tl, , br] = corners;
    for (let y = 0; y < w; y++) {
      for (let x = 0; x < w; x++) {
        const inside = x >= tl[0] && x <= br[0] && y >= tl[1] && y <= br[1];
        const v = inside ? inner : outer;
        const i = (y * w + x) * 4;
        rgba[i] = v;
        rgba[i + 1] = v;
        rgba[i + 2] = v;
        rgba[i + 3] = 255;
      }
    }
    return rgba;
  }

  const corners: [Point2, Point2, Point2, Point2] = [
    [20, 20],
    [44, 20],
    [44, 44],
    [20, 44],
  ];

  it("reads dark-inside/light-outside as NOT inverted (normal polarity)", () => {
    const rgba = makeFrame(corners, 30, 220);
    const probe = probeInvertedFromRgba(rgba, 64, 64, corners, 4);
    expect(probe.inverted).toBe(false);
    expect(probe.lumaInside).toBe(30);
    expect(probe.lumaOutside).toBe(220);
    expect(probe.lowContrast).toBe(false);
  });

  it("reads light-inside/dark-outside as inverted", () => {
    const rgba = makeFrame(corners, 220, 30);
    const probe = probeInvertedFromRgba(rgba, 64, 64, corners, 4);
    expect(probe.inverted).toBe(true);
    expect(probe.contrast).toBe(190);
  });

  it("flags low contrast below the 30-delta threshold", () => {
    const rgba = makeFrame(corners, 110, 128);
    const probe = probeInvertedFromRgba(rgba, 64, 64, corners, 4);
    expect(probe.contrast).toBe(18);
    expect(probe.lowContrast).toBe(true);
  });

  it("does not flag contrast at/above the threshold", () => {
    const rgba = makeFrame(corners, 90, 128);
    const probe = probeInvertedFromRgba(rgba, 64, 64, corners, 4);
    expect(probe.contrast).toBe(38);
    expect(probe.lowContrast).toBe(false);
  });

  it("throws when a probe would land outside the frame (corner at the image edge)", () => {
    const edgeCorners: [Point2, Point2, Point2, Point2] = [
      [0, 0],
      [44, 0],
      [44, 44],
      [0, 44],
    ];
    const rgba = makeFrame(edgeCorners, 30, 220);
    // The OUTSIDE probe from TL=(0,0) steps to negative coordinates.
    expect(() => probeInvertedFromRgba(rgba, 64, 64, edgeCorners, 4)).toThrow(RangeError);
  });

  it("rejects a non-positive module size and degenerate corners", () => {
    const rgba = makeFrame(corners, 30, 220);
    expect(() => probeInvertedFromRgba(rgba, 64, 64, corners, 0)).toThrow(RangeError);
    const degenerate: [Point2, Point2, Point2, Point2] = [
      [20, 20],
      [20, 20],
      [20, 20],
      [20, 20],
    ];
    expect(() => probeInvertedFromRgba(rgba, 64, 64, degenerate, 4)).toThrow(RangeError);
  });
});
