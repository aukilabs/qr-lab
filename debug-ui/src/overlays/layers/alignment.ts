// Alignment-pattern overlay (Plan 4 Task 6): a gray × at every searched
// lattice slot's PREDICTED position, plus a green ● at its FOUND position
// when the concentric probe re-centered one. Draws from
// `scan.trace.alignment` (`AlignmentTraceEntry[]`, working-res image px —
// the same space `scan.detections`' own geometry lives in, no
// `workingScale` conversion needed, unlike the `groundtruth` layer's
// SOURCE-px `corners_px`). Only ever non-empty when a trace was captured
// (`scan.trace !== null`) AND the last attempted candidate's version has at
// least one non-finder-corner alignment-pattern slot (v1 always yields an
// empty array — see the Rust `trace::AlignmentTraceEntry` doc).
//
// CAVEAT: "last attempted candidate" is NOT necessarily the same candidate
// `scan.detections.codes` or the `bits`/`decoded` layers describe — in a
// multi-triplet frame, the last attempt run can be a different (and
// possibly failed) triplet from whichever one(s) actually decoded, e.g. a
// spurious finder-noise triplet attempted after the real code already
// decoded. Don't assume this layer's markers describe the same physical
// code as a `decoded` badge visible elsewhere on the canvas — see the Rust
// `trace::Trace::alignment` doc for the full contract.
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
