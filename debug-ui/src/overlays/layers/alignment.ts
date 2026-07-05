// Alignment-pattern overlay (Plan 4 Task 6): a gray × at every searched
// lattice slot's PREDICTED position, plus a green ● at its FOUND position
// when the concentric probe re-centered one. Entries are
// `AlignmentTraceEntry[]`, working-res image px — the same space
// `scan.detections`' own geometry lives in, no `workingScale` conversion
// needed, unlike the `groundtruth` layer's SOURCE-px `corners_px`. Only
// ever non-empty when a trace was captured (`scan.trace !== null`) AND the
// describing candidate's version has at least one non-finder-corner
// alignment-pattern slot (v1 always yields an empty array — see the Rust
// `trace::AlignmentTraceEntry` doc).
//
// WHICH CANDIDATE(S) (Plan 5C: multi-code trace): `scan.trace.codes`
// carries one entry PER DECODED code this frame (see
// `qrk_core::trace::Trace::codes`'s doc) — this layer draws every entry's
// alignment search, so a multi-code scene shows all of them, not just one.
// Only when NOTHING decoded this frame (`codes` empty) does it fall back
// to the legacy singular `scan.trace.alignment` field, which then holds
// the FIRST attempt's search — canonical (unrotated) corner roles of the
// first (lowest-`snap_error`) candidate, never a rotation retry and never
// a later candidate (the Plan 4B Fix A failure-diagnosis rule, unchanged;
// pre-Fix-A that field tracked the LAST attempted candidate, frequently a
// wrong-role rotation retry whose geometry pointed away from the real
// code, misleading this overlay on every failed frame). Pre-Plan-5C the
// singular field was this layer's ONLY source — always one candidate's
// data even in a multi-code frame; see the Rust `trace::Trace::codes`/
// `Trace::alignment` docs for the full contract.
import { imageToScreen } from "../../viewport/transform";
import type { ViewTransform } from "../../viewport/transform";
import type { AlignmentTraceEntry } from "../../scanner/types";
import type { OverlayLayer } from "../registry";

const PREDICTED_COLOR = "#9e9e9e"; // gray
const FOUND_COLOR = "#00c853"; // green
const MARKER_HALF = 4; // screen px

function drawEntries(
  ctx: CanvasRenderingContext2D,
  view: ViewTransform,
  entries: AlignmentTraceEntry[],
): void {
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
}

export const alignmentLayer: OverlayLayer = {
  id: "alignment",
  label: "Alignment patterns",
  defaultEnabled: false,
  draw({ ctx, view, scan }) {
    const codes = scan?.trace?.codes;
    const fallback = scan?.trace?.alignment;
    if (!codes && !fallback) return;

    ctx.lineWidth = 1.5;

    if (codes && codes.length > 0) {
      for (const code of codes) {
        drawEntries(ctx, view, code.alignment);
      }
    } else if (fallback && fallback.length > 0) {
      drawEntries(ctx, view, fallback);
    }
  },
};
