import { describe, expect, it } from "vitest";
import { parseGroundTruth } from "./groundtruth-types";

// Trimmed to the fields this module cares about, but numerically taken
// straight from fixtures/near_06.json's single code entry — a real
// Rust-generated golden-fixture shape, not an invented one.
function validCode(): unknown {
  return {
    corners_px: [
      [504.5294838846006, 464.1528775470801],
      [609.5535908319368, 416.1761174073898],
      [657.1858134502476, 520.823469416443],
      [552.2874366800725, 569.221689460708],
    ],
    distance_m: 1.322308176708906, // extra fixture field this module ignores
    ecc: "m",
    inplane_deg: 335.51760760656464,
    inverted: false,
    mirrored: false,
    module_size_px: 5.498264528005626,
    opaque_plate: true,
    payload: "Q:near_06:0",
    physical_size_m: 0.15,
    tilt_azimuth_deg: 237.6861831033669,
    tilt_deg: 1.9078859714731622,
    version: 1,
  };
}

describe("parseGroundTruth", () => {
  it("accepts a real fixture code entry and extracts the fields it needs", () => {
    const parsed = parseGroundTruth(validCode());
    expect(parsed).toEqual({
      corners_px: [
        [504.5294838846006, 464.1528775470801],
        [609.5535908319368, 416.1761174073898],
        [657.1858134502476, 520.823469416443],
        [552.2874366800725, 569.221689460708],
      ],
      version: 1,
      module_size_px: 5.498264528005626,
      inverted: false,
      opaque_plate: true,
      payload: "Q:near_06:0",
    });
  });

  it("ignores extra fixture fields it doesn't need (mirrored, ecc, distance_m, ...)", () => {
    const raw = validCode() as Record<string, unknown>;
    const parsed = parseGroundTruth(raw);
    expect(parsed).not.toHaveProperty("mirrored");
    expect(parsed).not.toHaveProperty("ecc");
  });

  it("throws with a path when corners_px is missing", () => {
    const raw = validCode() as Record<string, unknown>;
    delete raw.corners_px;
    expect(() => parseGroundTruth(raw)).toThrowError(/corners_px/);
  });

  it("throws when corners_px doesn't have exactly 4 points", () => {
    const raw = validCode() as Record<string, unknown>;
    raw.corners_px = [
      [0, 0],
      [1, 1],
      [2, 2],
    ];
    expect(() => parseGroundTruth(raw)).toThrowError(/corners_px/);
  });

  it("throws with a path when a corner isn't a 2-element pair", () => {
    const raw = validCode() as Record<string, unknown>;
    (raw.corners_px as unknown[])[1] = [1, 2, 3];
    expect(() => parseGroundTruth(raw)).toThrowError(/corners_px\[1\]/);
  });

  it("throws with a path when version has the wrong type", () => {
    const raw = validCode() as Record<string, unknown>;
    raw.version = "1";
    expect(() => parseGroundTruth(raw)).toThrowError(/version/);
  });

  it("throws with a path when inverted has the wrong type", () => {
    const raw = validCode() as Record<string, unknown>;
    raw.inverted = "false";
    expect(() => parseGroundTruth(raw)).toThrowError(/inverted/);
  });

  it("throws when the value isn't an object", () => {
    expect(() => parseGroundTruth(null)).toThrowError(/GroundTruthCode/);
    expect(() => parseGroundTruth("nope")).toThrowError(/GroundTruthCode/);
  });
});
