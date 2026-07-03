// Pure view-transform math for the debug UI's canvas viewport. No React, no
// canvas/DOM APIs — this module only relates image pixel coordinates to
// screen (CSS-pixel, device-pixel-ratio-agnostic) coordinates so it stays
// trivially unit-testable and reusable from the overlay layers (Task 4),
// which project data through `imageToScreen` to draw on top of the base
// bitmap.

/** screen = image * scale + t (same `scale` on both axes — no independent
 * x/y stretch, since the debug UI never needs non-uniform zoom). */
export interface ViewTransform {
  scale: number;
  tx: number;
  ty: number;
}

export const identity: ViewTransform = { scale: 1, tx: 0, ty: 0 };

// Zoom clamp range. 64x shows a single ~2px module at 128px on screen —
// already well past useful pixel-level inspection. 0.05x fits a 24MP photo
// (e.g. 6000x4000) inside a typical laptop viewport without the image
// collapsing to sub-pixel size.
const MIN_SCALE = 0.05;
const MAX_SCALE = 64;

function clampScale(scale: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

export function imageToScreen(t: ViewTransform, p: [number, number]): [number, number] {
  return [p[0] * t.scale + t.tx, p[1] * t.scale + t.ty];
}

export function screenToImage(t: ViewTransform, p: [number, number]): [number, number] {
  return [(p[0] - t.tx) / t.scale, (p[1] - t.ty) / t.scale];
}

/**
 * Zoom by `factor` (>1 zooms in, <1 zooms out) while keeping the image
 * point currently under `screenPoint` fixed on screen — the standard
 * "zoom toward the cursor" behavior. Resulting scale is clamped to
 * `[MIN_SCALE, MAX_SCALE]`; the anchor stays fixed even when clamped,
 * since the translation is re-derived from the (possibly clamped) scale.
 */
export function zoomAt(
  t: ViewTransform,
  screenPoint: [number, number],
  factor: number,
): ViewTransform {
  const anchorImage = screenToImage(t, screenPoint);
  const scale = clampScale(t.scale * factor);
  return {
    scale,
    tx: screenPoint[0] - anchorImage[0] * scale,
    ty: screenPoint[1] - anchorImage[1] * scale,
  };
}

/** Translate by a screen-space delta; scale is untouched. Composes
 * additively: `pan(pan(t, a, b), c, d) === pan(t, a + c, b + d)`. */
export function pan(t: ViewTransform, dx: number, dy: number): ViewTransform {
  return { scale: t.scale, tx: t.tx + dx, ty: t.ty + dy };
}

/**
 * The transform that centers an `imgW x imgH` image inside a
 * `viewW x viewH` viewport at the largest scale that keeps the whole image
 * visible ("contain" fit), letterboxing the shorter axis. Falls back to
 * `identity` for degenerate (non-positive) dimensions rather than dividing
 * by zero / producing NaN.
 */
export function fitToView(
  imgW: number,
  imgH: number,
  viewW: number,
  viewH: number,
): ViewTransform {
  if (imgW <= 0 || imgH <= 0 || viewW <= 0 || viewH <= 0) {
    return identity;
  }
  // The contain-scale goes through the same [MIN_SCALE, MAX_SCALE] clamp
  // as interactive zoom. Trade-off: an image so large that fitting needs
  // scale < MIN_SCALE (e.g. a 100000px-wide strip into a 100px view) gets
  // clipped instead of fully shown — accepted, since a consistent zoom
  // floor matters more than pathological aspect ratios the debug UI never
  // feeds it (a 24MP photo still fits at 0.05).
  const scale = clampScale(Math.min(viewW / imgW, viewH / imgH));
  return {
    scale,
    tx: (viewW - imgW * scale) / 2,
    ty: (viewH - imgH * scale) / 2,
  };
}
