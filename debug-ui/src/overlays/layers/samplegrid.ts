// Sample-region overlay (Plan 4 Task 6): the 4-corner outline of every
// region `crates/qrk-core/src/sample.rs` tiled the module grid into for the
// last attempted candidate — region borders only, no per-module lines (per
// the plan's trace-compactness constraint: `SampleRegionTrace` carries
// corner quads, not per-module points, and this layer keeps draw counts
// sane at v40's up-to-36 regions by never expanding that back into a
// per-module grid). Draws from `scan.trace.sample_regions`, working-res
// image px (same space as `scan.detections`' own geometry — no
// `workingScale` conversion, unlike `groundtruth`'s SOURCE-px corners).
//
// CAVEAT: same "last ATTEMPTED, not last DECODED" contract as the
// `alignment` layer — this can describe a different, unrelated (possibly
// failed) candidate than whichever code(s) the `bits`/`decoded` layers are
// currently showing. See the Rust `trace::Trace::sample_regions` doc.
import { imageToScreen } from "../../viewport/transform";
import type { OverlayLayer } from "../registry";

const COLOR = "#ffca28"; // amber

export const samplegridLayer: OverlayLayer = {
  id: "samplegrid",
  label: "Sample regions",
  defaultEnabled: false,
  draw({ ctx, view, scan }) {
    const regions = scan?.trace?.sample_regions;
    if (!regions || regions.length === 0) return;

    ctx.strokeStyle = COLOR;
    ctx.lineWidth = 1;
    for (const region of regions) {
      const quad = region.quad.map((p) => imageToScreen(view, p));
      ctx.beginPath();
      ctx.moveTo(quad[0]![0], quad[0]![1]);
      for (let i = 1; i < quad.length; i++) {
        ctx.lineTo(quad[i]![0], quad[i]![1]);
      }
      ctx.closePath();
      ctx.stroke();
    }
  },
};
