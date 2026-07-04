// Alignment-pattern overlay (Plan 4 Task 6): a gray × at every searched
// lattice slot's PREDICTED position, plus a green ● at its FOUND position
// when the concentric probe re-centered one. Draws from
// `scan.trace.alignment` (`AlignmentTraceEntry[]`, working-res image px —
// the same space `scan.detections`' own geometry lives in, no
// `workingScale` conversion needed, unlike the `groundtruth` layer's
// SOURCE-px `corners_px`). Only ever non-empty when a trace was captured
// (`scan.trace !== null`) AND the describing candidate's version has at
// least one non-finder-corner alignment-pattern slot (v1 always yields an
// empty array — see the Rust `trace::AlignmentTraceEntry` doc).
//
// WHICH CANDIDATE (Plan 4B Fix A — trace honesty): when any candidate
// decoded this frame, this is THAT candidate's alignment search — it then
// always agrees with the `bits`/`decoded` layers and with
// `scan.detections.codes`' own last entry. When NOTHING decoded this frame,
// it's the FIRST attempt run — canonical (unrotated) corner roles of the
// first (lowest-`snap_error`) candidate, never a rotation retry and never a
// later candidate. Pre-Fix-A this field tracked the LAST attempted
// candidate instead, which — after a failed candidate's corner-role
// rotation retries — was frequently a wrong-role attempt whose geometry
// pointed away from the real code, misleading this overlay on every failed
// frame; see the Rust `trace::Trace::alignment` doc for the full contract.
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";

const PREDICTED_COLOR = "#9e9e9e"; // gray
const FOUND_COLOR = "#00c853"; // green
const MARKER_HALF = 4; // screen px

export const alignmentLayer: OverlayLayer = {
  id: "alignment",
  label: "Alignment patterns",
  defaultEnabled: false,
  draw({ ctx, view, scan }) {
    const entries = scan?.trace?.alignment;
    if (!entries || entries.length === 0) return;

    ctx.lineWidth = 1.5;
    for (const entry of entries) {
      const [px, py] = imageToScreen(view, entry.predicted);
      ctx.strokeStyle = PREDICTED_COLOR;
      ctx.beginPath();
      ctx.moveTo(px - MARKER_HALF, py - MARKER_HALF);
      ctx.lineTo(px + MARKER_HALF, py + MARKER_HALF);
      ctx.moveTo(px + MARKER_HALF, py - MARKER_HALF);
      ctx.lineTo(px - MARKER_HALF, py + MARKER_HALF);
      ctx.stroke();

      if (entry.found) {
        const [fx, fy] = imageToScreen(view, entry.found);
        ctx.fillStyle = FOUND_COLOR;
        ctx.beginPath();
        ctx.arc(fx, fy, MARKER_HALF, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  },
};
