/** Map a point in frame pixel space into the preview view (cover / FILL_CENTER). */
export function mapFramePointToView(
  x: number,
  y: number,
  frameW: number,
  frameH: number,
  viewW: number,
  viewH: number,
): { x: number; y: number } {
  if (frameW <= 0 || frameH <= 0 || viewW <= 0 || viewH <= 0) {
    return { x: 0, y: 0 };
  }
  const frameAspect = frameW / frameH;
  const viewAspect = viewW / viewH;
  let scale: number;
  let offsetX = 0;
  let offsetY = 0;
  if (viewAspect > frameAspect) {
    // View is wider → fill width, crop top/bottom.
    scale = viewW / frameW;
    offsetY = (viewH - frameH * scale) / 2;
  } else {
    // View is taller → fill height, crop sides.
    scale = viewH / frameH;
    offsetX = (viewW - frameW * scale) / 2;
  }
  return {
    x: x * scale + offsetX,
    y: y * scale + offsetY,
  };
}

/** Working-px corner → source-px via sourceScale (width-axis). */
export function workingToSource(
  corner: [number, number],
  sourceScale: number,
): [number, number] {
  if (sourceScale === 0) return corner;
  return [corner[0] / sourceScale, corner[1] / sourceScale];
}
