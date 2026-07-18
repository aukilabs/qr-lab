// Triplet-candidate overlay: distinguishable markers at each corner (TL =
// square, TR = triangle, BL = circle — matches the `tl`/`tr`/`bl` naming so
// a reader can tell which finder played which role without a legend), leg
// lines connecting them, and (winners only) a `dim=N (±snap_error)` label.
// Draws from `scan.detections.triplets` (always populated; see
// `finders.ts`'s comment on why trace isn't required). Marker sizes are
// constant screen px (not image-proportional) so they stay legible at any
// zoom level.
//
// WINNER vs CANDIDATE (Plan 5C): `scan.detections.triplets` includes every
// grouped triplet candidate, on purpose — decode arbitration resolves
// overlapping/cross-code phantoms later, and the trace stays honest by not
// hiding them (see `qr_lab_core::triplet`'s doc). In a multi-code scene those
// phantoms visually "link" unrelated codes together with solid lime legs,
// same as a real winner — misleading at a glance. This layer instead
// distinguishes: a WINNER (its `finder_indices` match a decoded
// `DecodedCode.finder_indices`, order-independent — see `finderKey`) draws
// solid and fully opaque with its dimension label, exactly as before;
// every other (candidate/loser) triplet draws at reduced alpha with dashed
// legs/markers and no label — still visible for debugging, but visually
// subordinate to the winners. `ctx.setLineDash`/`ctx.globalAlpha` are reset
// via `save`/`restore` around each triplet so one candidate's dashing can
// never bleed into the next triplet drawn.
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";

const COLOR = "#76ff03"; // lime
const TEXT_COLOR = "#ffffff";
const MARKER_HALF = 6; // screen px
const LABEL_FONT = "10px monospace";
const CANDIDATE_ALPHA = 0.35;
const CANDIDATE_DASH: [number, number] = [4, 3];

/** Sorted, comma-joined finder-index key so a triplet's finder SET can be
 * compared against a decoded code's `finder_indices` regardless of
 * corner-role assignment — `decode_candidates`'s cyclic corner-role
 * rotation retries (see that function's doc) reorder which finder plays
 * tl/tr/bl but never change the underlying set of three finders, so
 * comparing sorted keys is the correct (rotation-invariant) match. */
function finderKey(indices: readonly [number, number, number]): string {
  return [...indices].sort((a, b) => a - b).join(",");
}

export const tripletsLayer: OverlayLayer = {
  id: "triplets",
  label: "Triplets",
  defaultEnabled: true,
  draw({ ctx, view, scan }) {
    const triplets = scan?.detections.triplets;
    if (!triplets || triplets.length === 0) return;

    const decodedKeys = new Set(
      (scan?.detections.codes ?? []).map((c) => finderKey(c.finder_indices)),
    );

    ctx.lineWidth = 1.5;
    ctx.font = LABEL_FONT;

    for (const t of triplets) {
      const winner = decodedKeys.has(finderKey(t.finder_indices));
      const [tlx, tly] = imageToScreen(view, t.tl);
      const [trx, try_] = imageToScreen(view, t.tr);
      const [blx, bly] = imageToScreen(view, t.bl);

      ctx.save();
      ctx.globalAlpha = winner ? 1 : CANDIDATE_ALPHA;
      ctx.setLineDash(winner ? [] : CANDIDATE_DASH);
      ctx.strokeStyle = COLOR;

      // Leg lines: TL-TR and TL-BL.
      ctx.beginPath();
      ctx.moveTo(tlx, tly);
      ctx.lineTo(trx, try_);
      ctx.moveTo(tlx, tly);
      ctx.lineTo(blx, bly);
      ctx.stroke();

      // TL marker: square.
      ctx.strokeRect(tlx - MARKER_HALF, tly - MARKER_HALF, MARKER_HALF * 2, MARKER_HALF * 2);

      // TR marker: triangle (point up).
      ctx.beginPath();
      ctx.moveTo(trx, try_ - MARKER_HALF);
      ctx.lineTo(trx + MARKER_HALF, try_ + MARKER_HALF);
      ctx.lineTo(trx - MARKER_HALF, try_ + MARKER_HALF);
      ctx.closePath();
      ctx.stroke();

      // BL marker: circle.
      ctx.beginPath();
      ctx.arc(blx, bly, MARKER_HALF, 0, Math.PI * 2);
      ctx.stroke();

      // Dimension label: winners only — a candidate/loser triplet stays
      // visible via its markers/legs, but the label is reserved for the
      // triplet decode arbitration actually picked.
      if (winner) {
        ctx.fillStyle = TEXT_COLOR;
        ctx.fillText(`dim=${t.dimension} (±${t.snap_error.toFixed(2)})`, tlx + 4, tly - 8);
      }

      ctx.restore();
    }
  },
};
