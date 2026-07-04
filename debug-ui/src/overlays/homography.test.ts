import { describe, expect, it } from "vitest";
import { mapPoint, squareToQuad, type Point2 } from "./homography";

// Same Q used by crates/qrk-core/src/homography.rs's `#[cfg(test)] mod
// tests` — kept byte-identical so this file's assertions are checking the
// TS port against the Rust source of truth, not an independently invented
// fixture.
const Q: [Point2, Point2, Point2, Point2] = [
  [100.0, 50.0],
  [420.0, 80.0],
  [400.0, 380.0],
  [90.0, 350.0],
];

describe("squareToQuad / mapPoint", () => {
  it("maps the unit square's four corners exactly onto Q", () => {
    const h = squareToQuad(Q);
    expect(h).not.toBeNull();
    const uv: Point2[] = [
      [0.0, 0.0],
      [1.0, 0.0],
      [1.0, 1.0],
      [0.0, 1.0],
    ];
    uv.forEach(([u, v], i) => {
      const [px, py] = mapPoint(h!, u, v);
      const expected = Q[i]!;
      expect(px).toBeCloseTo(expected[0], 9);
      expect(py).toBeCloseTo(expected[1], 9);
    });
  });

  // Cross-checked against the real Rust implementation: a temporary
  // `#[test]` added to homography.rs (`h.map(0.3, 0.6)` with this same Q,
  // via `square_to_quad(Q).unwrap()`) printed
  // `187.894239848914083 241.246458923512762` before being reverted —
  // this pins the TS port against that independently-computed value
  // rather than against itself.
  it("maps an interior point to the value computed by the Rust implementation", () => {
    const h = squareToQuad(Q);
    expect(h).not.toBeNull();
    const [px, py] = mapPoint(h!, 0.3, 0.6);
    expect(px).toBeCloseTo(187.894239848914083, 9);
    expect(py).toBeCloseTo(241.246458923512762, 9);
  });

  it("scales linearly in the affine (parallelogram) branch", () => {
    // Axis-aligned square scaled 2x — same case as the Rust
    // `affine_case_scales` test, same hand-computable expected value
    // (0.25*2, 0.75*2).
    const q: [Point2, Point2, Point2, Point2] = [
      [0.0, 0.0],
      [2.0, 0.0],
      [2.0, 2.0],
      [0.0, 2.0],
    ];
    const h = squareToQuad(q);
    expect(h).not.toBeNull();
    const [px, py] = mapPoint(h!, 0.25, 0.75);
    expect(px).toBeCloseTo(0.5, 12);
    expect(py).toBeCloseTo(1.5, 12);
  });

  it("returns null for a collinear quad (degenerate projective branch)", () => {
    const q: [Point2, Point2, Point2, Point2] = [
      [0.0, 0.0],
      [1.0, 0.0],
      [2.0, 0.0],
      [3.0, 0.0],
    ];
    expect(squareToQuad(q)).toBeNull();
  });

  it("returns null for a zero-area parallelogram (degenerate affine branch)", () => {
    const q: [Point2, Point2, Point2, Point2] = [
      [0.0, 0.0],
      [2.0, 0.0],
      [2.0, 0.0],
      [0.0, 0.0],
    ];
    expect(squareToQuad(q)).toBeNull();
  });
});
