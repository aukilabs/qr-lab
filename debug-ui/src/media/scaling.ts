// Source-image px -> working-res px scale factor, matching
// `overlays/registry.ts`'s `OverlayContext.workingScale` contract
// (`workingWidth / sourceWidth`) exactly — this is the one place App.tsx
// computes that factor, so it's worth its own tiny testable module rather
// than an inline expression duplicated at each call site (image mode's
// post-scan update and video mode's per-frame update both need it).
//
// Binding decision (Task 6 brief): the debug UI displays the DOWNSCALED
// (working-res) bitmap in the Viewport, not the original — so overlay
// layers that already consume working px (tiles/finders/triplets) need no
// scaling, and only the `groundtruth` layer (whose fixture data is in
// SOURCE px) multiplies by this factor before projecting through
// `imageToScreen`. See `overlays/registry.ts`'s `OverlayContext` doc
// comment for the full contract this factor plugs into.

/**
 * `scanWidth / sourceWidth` — the factor that converts a SOURCE-image px
 * coordinate (e.g. a golden fixture's `corners_px`) into a WORKING-res px
 * coordinate (the space `scan` results and the displayed bitmap live in).
 * Guards the degenerate `sourceWidth <= 0` case (no source loaded yet, or a
 * malformed image) by returning `1` (identity) rather than dividing by
 * zero / producing `NaN` — there's no meaningful scale factor with no
 * source, and `1` is a safer default than `NaN` propagating into every
 * overlay coordinate.
 */
export function workingScaleFor(sourceWidth: number, scanWidth: number): number {
  if (sourceWidth <= 0) return 1;
  return scanWidth / sourceWidth;
}
