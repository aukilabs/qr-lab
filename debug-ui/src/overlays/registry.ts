// Overlay framework: the fixed contract every debug-UI overlay layer (Task
// 4's tiles/finders/triplets/groundtruth, and any future stage's layer)
// draws through, plus the registry that owns enable/disable state and
// dispatches a redraw across all of them. See `Viewport.tsx`'s `overlays`
// prop for how a `drawAll` call gets wired to an actual canvas.
import type { ViewTransform } from "../viewport/transform";
import type { GroundTruthCode } from "./groundtruth-types";
import type { RobustDetections } from "../scanner/robust-types";
import type { ScanResult } from "../scanner/types";

export interface OverlayContext {
  /** Destination 2D context, already in screen (CSS-pixel) space — a
   * layer must project image-space coordinates through
   * `imageToScreen(view, ...)` itself before drawing; it must never draw
   * at raw image-pixel coordinates directly. */
  ctx: CanvasRenderingContext2D;
  /** Current pan/zoom transform: image px (working-res, see
   * `workingScale`) -> screen px. */
  view: ViewTransform;
  /** Latest scan result, in working-res px, or `null` before any scan has
   * run. */
  scan: ScanResult | null;
  /** Golden-fixture ground-truth codes, in SOURCE-image px, or `null`
   * when the current source has no ground truth (e.g. a real photo/video
   * instead of a golden fixture). Parsed by Task 6's source panel. */
  groundTruth: GroundTruthCode[] | null;
  /** Working-resolution image dimensions `[w, h]` — the space `scan`'s
   * coordinates live in. */
  imageSize: [number, number];
  /** source-image px -> working-res px factor (`workingWidth /
   * sourceWidth`; the debug UI never stretches width/height
   * independently, so one scalar covers both axes). `scan` is already in
   * working px and needs no further scaling. `groundTruth` (`corners_px`,
   * `module_size_px`, ...) is in SOURCE px — only the `groundtruth` layer
   * multiplies by this factor before projecting through `imageToScreen`;
   * every other layer consumes working px directly. */
  workingScale: number;
  /** Plan 6 robust-mode ladder result, or `null`/absent outside robust
   * mode (optional so pre-Plan-6 context constructors — including Scene3D
   * and the layer tests — keep compiling unchanged; absent means `null`).
   * Its geometry (`codes[i].corners_source`/`refined_corners_source`,
   * `triplet_evidence`) is SOURCE px — the robust layers multiply by
   * `workingScale` before projecting, same convention as `groundTruth`. */
  robust?: RobustDetections | null;
}

export interface OverlayLayer {
  /** Stable identifier — used as the `enabled` set key and passed to
   * `toggle`. */
  id: string;
  /** Human-readable name for `LayerPanel`'s checkbox list. */
  label: string;
  /** Whether this layer starts enabled in a freshly created registry. */
  defaultEnabled: boolean;
  /** Draw this layer's overlay onto `o.ctx`. Must be self-contained: bail
   * out (return without drawing) when its relevant data in `o` is absent,
   * rather than throwing — though `drawAll` also guards against a
   * throwing layer, so a bug here can't take down the other layers. */
  draw(o: OverlayContext): void;
}

export interface OverlayRegistry {
  /** All registered layers, in registration order — `LayerPanel` renders
   * its checkbox list in this order. */
  layers: OverlayLayer[];
  /** Mutated in place by `toggle`; membership means "currently drawn". Not
   * React state — callers that need a re-render on toggle (e.g.
   * `LayerPanel`) must force one themselves. */
  enabled: Set<string>;
  /** Flip `id`'s membership in `enabled` (enabled -> disabled and vice
   * versa). A no-op-safe operation on an unregistered `id` (it's simply
   * added to `enabled`, which then draws nothing since no layer matches
   * it) — the registry doesn't validate `id` against `layers`. */
  toggle(id: string): void;
  /** Draw every enabled layer, in registration order. A layer that throws
   * is caught so it can't stop the rest from drawing; its error is logged
   * to the console once per layer `id` (not on every redraw — `drawAll`
   * typically runs once per animation frame, so unthrottled logging would
   * flood the console for a persistently-broken layer). */
  drawAll(o: OverlayContext): void;
}

export function createRegistry(layers: OverlayLayer[]): OverlayRegistry {
  const enabled = new Set(layers.filter((l) => l.defaultEnabled).map((l) => l.id));
  const loggedErrors = new Set<string>();

  function toggle(id: string): void {
    if (enabled.has(id)) {
      enabled.delete(id);
    } else {
      enabled.add(id);
    }
  }

  function drawAll(o: OverlayContext): void {
    for (const layer of layers) {
      if (!enabled.has(layer.id)) continue;
      try {
        layer.draw(o);
      } catch (err) {
        if (!loggedErrors.has(layer.id)) {
          loggedErrors.add(layer.id);
          console.error(`overlay layer "${layer.id}" threw during draw:`, err);
        }
      }
    }
  }

  return { layers, enabled, toggle, drawAll };
}
