// Sample-region overlay (Plan 4 Task 6): the 4-corner outline of every
// region `crates/qrk-core/src/sample.rs` tiled the module grid into for the
// describing candidate(s) (see below) — region borders only, no per-module
// lines (per the plan's trace-compactness constraint: `SampleRegionTrace`
// carries corner quads, not per-module points, and this layer keeps draw
// counts sane at v40's up-to-36 regions by never expanding that back into a
// per-module grid). Draws from `scan.trace.codes[*].sample_regions`,
// working-res image px (same space as `scan.detections`' own geometry — no
// `workingScale` conversion, unlike `groundtruth`'s SOURCE-px corners).
//
// WHICH CANDIDATE(S) (Plan 5C: multi-code trace): `scan.trace.codes`
// carries one entry PER DECODED code this frame (see
// `qrk_core::trace::Trace::codes`'s doc) — this layer draws every entry's
// regions, so a multi-code scene (e.g. `multi_07`'s 4 codes) shows all of
// them, not just one. Only when NOTHING decoded this frame (`codes` empty)
// does it fall back to the legacy singular `scan.trace.sample_regions`
// field, which then holds the FIRST attempt's canonical geometry
// (failure-diagnosis only — see `Trace`'s doc) — Pre-Plan-5C this was the
// layer's ONLY source, always a single candidate's data (Plan 4B Fix A: the
// most recently decoded one), so a multi-code frame only ever visualized
// one of its several decoded codes.
import { imageToScreen } from "../../viewport/transform";
import type { ViewTransform } from "../../viewport/transform";
import type { SampleRegionTrace } from "../../scanner/types";
import type { OverlayLayer } from "../registry";

const COLOR = "#ffca28"; // amber

function drawRegions(ctx: CanvasRenderingContext2D, view: ViewTransform, regions: SampleRegionTrace[]): void {
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
}

export const samplegridLayer: OverlayLayer = {
  id: "samplegrid",
  label: "Sample regions",
  defaultEnabled: false,
  draw({ ctx, view, scan }) {
    const codes = scan?.trace?.codes;
    const fallback = scan?.trace?.sample_regions;
    if (!codes && !fallback) return;

    ctx.strokeStyle = COLOR;
    ctx.lineWidth = 1;

    if (codes && codes.length > 0) {
      for (const code of codes) {
        drawRegions(ctx, view, code.sample_regions);
      }
    } else if (fallback && fallback.length > 0) {
      drawRegions(ctx, view, fallback);
    }
  },
};
