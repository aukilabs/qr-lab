import { describe, expect, it } from "vitest";
import {
  buildFixtureMeta,
  defaultFixtureName,
  eccLetterFromIndex,
  opaquePlateFromAlpha,
  slugifyPayload,
  versionFromDim,
} from "./fixtureExport";
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
