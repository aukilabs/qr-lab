# qrk debug UI

A React + Vite app for visually driving the `qrk-core` QR scanner (compiled
to WASM via `qrk-wasm`) against golden fixtures, real photos, and arbitrary
dropped images/videos — with every detection stage rendered as an
independently togglable overlay and a live per-stage timings panel. This is
a development tool, not a product: plain CSS, no component library, no
router.

## Setup

From the repo root, the WASM package must be built once (and rebuilt any
time `crates/qrk-core`/`crates/qrk-wasm` change) before the app can run —
`predev`/`prebuild` check for it and fail with a pointer to this command if
it's missing:

```bash
npm run build:wasm      # wasm-pack build -> debug-ui/src/wasm/ (release, wasm-opt'd)
```

Then, from `debug-ui/`:

```bash
npm install
npm run dev              # vite dev server, default http://localhost:5173/
npm test                 # vitest run — logic modules (downscale, transform,
                          #   overlays/registry, timings-model, envelope
                          #   parsing, scanner client) are unit-tested;
                          #   DOM-heavy hooks (useImageSource/useVideoSource)
                          #   and components are manual-QA'd instead — see
                          #   `.superpowers/sdd/task-7-report.md` for the
                          #   executed checklist.
npm run build             # tsc --noEmit && vite build
```

Fixtures are served from `debug-ui/public/fixtures`, a symlink to
`../../fixtures` (checked into git as a symlink; both `vite dev` and `vite
build` follow it — a build copies the real file contents into `dist/`, so
the symlink itself isn't needed at runtime). `predev`/`prebuild` also
regenerate `public/fixtures-manifest.json` (gitignored) from whatever is
currently on disk in `fixtures/` — see `scripts/gen-fixture-manifest.mjs`.

A rebuilt WASM package is picked up on the **next full page reload** — the
worker imports `src/wasm/qrk_wasm.js` once at startup, so `npm run
build:wasm` followed by a browser reload (not just a fixture change) is
required to pick up new Rust code while `npm run dev` keeps running.

## Architecture

```
                     ┌─────────────┐
  fixture manifest → │ SourcePanel │ → SourceDescriptor (fixture | dropped file, image | video)
  drag-drop / pick   └─────────────┘
                             │
                             ▼
                 useImageSource / useVideoSource   (media/*.ts)
                   decode → Uint8ClampedArray rgba
                             │
                             ▼
                       App.runScan(rgba, w, h)
                       ├─ downscaleRgba (display path only, since Plan 5:
                       │    builds the Viewport bitmap; see App.tsx's
                       │    module doc comment)
                       ▼
                  ScannerClient.scan(rgba, w, h, {maxDim, refine})
                       →  scanner/worker.ts (Web Worker), full-res rgba
                       │     scan_rgba() from qrk-wasm — luma + the NN
                       │     downscale now happen in Rust (qrk_core::scan)
                       ▼
                  ScanResult { detections: { finders, triplets, codes,
                                              timings, source_scale },
                               trace: { tiles, finders, triplets, attempts,
                                        alignment, sample_regions, bits } | null }
                       │
          ┌────────────┼─────────────────────┐
          ▼            ▼                     ▼
     Viewport    OverlayRegistry        TimingsPanel
   (pan/zoom,     .drawAll(ctx, view,    (per-stage µs/ms +
    2 canvases:    scan, groundTruth,     rolling sparkline;
    bitmap +       imageSize,             timings-model.ts)
    overlay)       workingScale)
                       ▲
                       │ registered once, module scope
                tilesLayer, findersLayer, tripletsLayer,
                groundtruthLayer, alignmentLayer,
                samplegridLayer, bitsLayer, decodedLayer,
                refinedLayer
                (overlays/layers/*.ts)
                       ▲
                LayerPanel (checkboxes mirror
                registry.enabled; toggling
                forces a Viewport redraw)
```

Key modules:

- `scanner/types.ts` — the TS mirror of the WASM envelope. **Cross-language
  contract**: field names come from
  `scanner/__snapshots__/envelope.near_00.json`, a Rust-generated snapshot
  (`crates/qrk-wasm/tests/envelope_snapshot.rs`), not from guessing. If
  `WasmResult`'s Rust shape changes, regenerate the snapshot
  (`UPDATE_SNAPSHOT=1 cargo test -p qrk-wasm --test envelope_snapshot`) and
  update `parseScanResult` to match — `envelope.test.ts` fails loudly on
  drift. **Gotcha (found by Plan 4 Task 7's live-browser QA, fixed in the
  same task):** the snapshot is JSON text (`serde_json`), which renders a
  Rust `Option::None` as `null` — but the REAL `scan_rgba` binding
  (`serde_wasm_bindgen::to_value`, `qrk-wasm/src/lib.rs`) renders `None` as
  `undefined` instead (key present, value `undefined`), `serde-wasm-bindgen`'s
  documented default. Every "OrNull" parser in this file (`parseNumberOrNull`,
  `parsePairOrNull`, `parseBitsTraceOrNull`, `parseTileTraceOrNull`,
  `parseTraceOrNull`) treats both the same way — if a new optional field's
  parser only checks `=== null`, it will throw on every real scan where that
  `Option` is `None` (e.g. `version_bits` for any code below version 7),
  while `envelope.test.ts` stays green (it only exercises the JSON-shaped
  snapshot). Add an `undefined` regression test alongside the `null` one, not
  just the latter.
- `scanner/client.ts` / `scanner/worker.ts` — main-thread request/response
  wrapper around the Worker; "latest-wins" queueing so video mode can fire
  a scan per presented frame without an unbounded backlog when frames
  arrive faster than `scan_rgba` can process them. `ScanOptions.refine`
  defaults to `false` in `client.ts` (existing callers unaffected), but
  **media mode always passes `refine: true`** in `App.tsx`'s `runScan`
  (Plan 5 Task 7 QA fix — before this, media mode never enabled it at
  all, so `refinedLayer` and the timings panel's `refine` row were
  permanently dead there; Scene3D already hardcoded `refine: true` the
  same way). Cost is ~0.1-0.4ms/frame per Task 6's re-baseline —
  negligible for a dev tool with no UI toggle for it.
- `viewport/transform.ts` + `viewport/Viewport.tsx` — pan/zoom math and the
  two-canvas (bitmap + overlay) stack. All overlay coordinates are
  working-resolution image px; `imageToScreen(view, ...)` is the *only*
  sanctioned way to project into screen space (see the 5-step recipe
  below).
- `overlays/registry.ts` + `overlays/layers/*.ts` — the overlay framework
  (see "Add an overlay layer" below).
- `panels/SourcePanel.tsx`, `panels/LayerPanel.tsx`,
  `panels/TimingsPanel.tsx` — sidebar UI; `App.tsx` is the thin shell wiring
  all of the above together plus the video/image mode split.

## Mode 1: 3D orbit scene (Plan 5 Task 5)

A second top-level mode (the "3D Scene" tab, next to "Media") — an
orbitable react-three-fiber scene rendering a plane textured with a real,
generated, decodable QR code, whose module-region corners are known
analytically (via the plane's own transform + camera) and compared each
frame against the live `refined_corners` the same scanner produces for
media mode. This is the plan's headline debug-UI feature: a live subpixel-
accuracy meter you can orbit around.

- `scene3d/Scene3D.tsx` — the scene itself. `<OrbitControls>` drives the
  camera; every ~100ms (`SCAN_THROTTLE_MS`) the current view renders to an
  offscreen `THREE.WebGLRenderTarget` (NOT the on-screen canvas —
  `preserveDrawingBuffer` would cost every frame; see the file's doc
  comment), is read back as rgba, run through the camera-sim knobs, and
  handed to the SAME `ScannerClient` media mode uses (`refine: true`,
  hardcoded — no UI toggle). The on-screen `<Canvas>` renders the scene
  normally and independently of this readback — **the visible WebGL
  canvas never reflects the blur/noise/exposure knobs**, only the
  internal scan buffer does (a QA gotcha: don't expect the screenshot to
  look blurred/noisy when those sliders are up).
- `scene3d/moduleRegion.ts` + `scene3d/projection.ts` — project the
  plane's module-region corners through the same three.js camera each
  frame; this is the frame's ground truth, compared against
  `refined_corners` by `errorStats.ts`.
- `scene3d/camSim.ts` — the camera-sim knobs, each an explicitly
  documented approximation: render resolution (640/960/1280, the readback
  buffer size), Gaussian blur (canvas `ctx.filter`), sensor noise (seeded
  per-pixel Gaussian), exposure offset (flat add). None of these model a
  real lens/sensor; they're deliberately simple.
- `scene3d/ErrorPanel.tsx` — per-corner `|refined - truth|` in px this
  frame, plus a 60-sample rolling mean/p95 sparkline (mirrors
  `TimingsPanel`'s pattern).
- `scene3d/qrTexture.ts` — canvas-renders the wasm-generated bit matrix
  (`qrk-wasm`'s `qr-gen` feature, `scanner/qrgen.ts`) into a
  `CanvasTexture`; `NearestFilter` (mag) for sharp edges,
  `LinearMipMapLinearFilter` (min) to avoid aliasing under keystone.

**QA findings (Plan 5 Task 7), for anyone re-measuring this scene:**

- At the scene's literal default (head-on, `physicalSize=0.15m`,
  `resolution=960`) view, mean corner error sits around **1.0-1.1px**, not
  strictly sub-pixel — TR/BL corners run higher (~1.6-1.7px) than TL/BR
  (~0.4-0.5px), a directional bias most likely from the QR texture's own
  discretization/mip-sampling rather than the refinement algorithm itself
  (the equivalent *fixture* gate — real rendered images, no live texture
  approximation — locks at 0.10px; see the root README's Plan 5 section).
  Across 7 orbit poses spanning ~0-55° combined azimuth/polar, mean error
  stayed in a **0.6-1.05px band** (no monotonic growth) before dropping
  out cleanly (no garbage values — the error panel just reads all-`n/a`)
  at a grazing angle. This is a real, reasonably flat robustness profile,
  just not literally sub-pixel at this default framing — the
  `physicalSize`/distance/resolution combination that would tighten it
  further hasn't been swept.
- Blur (0→3px sigma) and noise (0→8σ) sweeps at the default framing
  produced a **flat** error curve (blur: 0.77-1.05px; noise: 1.02-1.06px)
  — decode never dropped anywhere in either range. At this scene's
  ~26px/module density (960px readback / 37 modules), both knob ranges
  stay inside the refinement's own outlier-rejection tolerance (Task 3's
  weighted-TLS refit). A smaller `physicalSize` or a higher QR version
  (denser grid, fewer px/module) would show the expected "error rises,
  then decode drops" curve much sooner — worth trying if this scene is
  used to demonstrate degradation, not just robustness.
- Render-resolution changes (1280→640, exercising `ScanScratch`'s
  realloc path) work cleanly — no console errors, decode continues at
  every step.
- `TimingsPanel`'s ms-resolution `StageClock` (`js_sys::Date::now()`)
  means the `refine` row (and `triplets`/`version`/`alignment`/
  `sample+decode` on fast fixtures) very often reads "n/a" here too — every
  fixture measured in Task 6's host re-baseline has `refine_ns` under 1ms
  (74-370us), so the browser's ms-granularity clock reads it as exactly
  `0` and `formatNs` renders that as "n/a" (documented behavior, not a
  bug — see `panels/TimingsPanel.tsx`'s doc comment). The refined-corner
  overlay itself (crosshairs + per-corner error labels) is the reliable
  way to confirm refinement actually ran; see
  `.superpowers/sdd/task-7-report.md` for the full QA record.

## Known limitations

`IMG_4832.png` (and any other EXIF-rotated photo) decodes on the Rust host
gates but not in this browser UI: Chrome's `createImageBitmap` honors EXIF
orientation, so the browser scans different pixels than the host does.
Production mobile receives raw camera Y-planes with no EXIF pathway, so
this is a dev-tool-only divergence — follow-up recorded in the Plan 5 doc's
Post-merge follow-ups section.

## Add an overlay layer (5-step recipe)

Follow this to add a new debug overlay for a future detection stage (Plan 4
Task 6 added `alignment.ts`/`samplegrid.ts`/`bits.ts`/`decoded.ts` for the
decode pipeline's own stages this same way, and Plan 5 Task 4 added
`refined.ts` for subpixel corner refinement — read one of those, or the
original `tiles.ts`/`finders.ts`/`triplets.ts`/`groundtruth.ts`, alongside
this list).

1. **Extend the contract, if the new stage needs new envelope fields.**
   Add the field(s) to the Rust `WasmResult`/`Trace`/`Detections` types,
   regenerate the snapshot (`UPDATE_SNAPSHOT=1 cargo test -p qrk-wasm
   --test envelope_snapshot`), then mirror the new field(s) in
   `scanner/types.ts` (interface + `parse*` function) so
   `envelope.test.ts` keeps passing. Skip this step if your layer only
   needs data that's already in `ScanResult`/`GroundTruthCode`.

2. **Write the layer module** at `overlays/layers/<name>.ts`, exporting an
   `OverlayLayer` (see `overlays/registry.ts` for the interface):
   ```ts
   export const myLayer: OverlayLayer = {
     id: "my-stage",              // stable key for registry.enabled + LayerPanel
     label: "My Stage",           // shown in the LayerPanel checkbox
     defaultEnabled: true,        // or false, e.g. tilesLayer's heatmap defaults off
     draw({ ctx, view, scan, groundTruth, imageSize, workingScale }) {
       // Bail out (return, don't throw) when your data isn't present yet
       // — see finders.ts/triplets.ts's early `if (!x) return;` pattern.
       // Project every image-space point through `imageToScreen(view, p)`
       // before drawing — never compute your own screen mapping (see
       // `OverlayContext`'s doc comment on why one shared transform matters).
     },
   };
   ```
   Add a unit test at `overlays/layers/<name>.test.ts` using
   `overlays/test-support/fake-canvas.ts` (a `CanvasRenderingContext2D`
   stub that records calls) — see `triplets.test.ts` for the pattern of
   asserting exact draw-call sequences.

3. **Register it** in `App.tsx`'s module-scope `createRegistry([...])` call
   (currently `[tilesLayer, findersLayer, tripletsLayer, groundtruthLayer,
   alignmentLayer, samplegridLayer, bitsLayer, decodedLayer, refinedLayer]`).
   Registration order is draw order (later entries draw on top) and
   `LayerPanel`'s checkbox order.

4. **Nothing else to wire manually** — `LayerPanel` renders every
   `registry.layers` entry automatically, `OverlayRegistry.drawAll` calls
   every enabled layer's `draw` each redraw (wrapped in its own try/catch,
   so a throwing layer can't take down the others), and `App.tsx`'s
   `handleOverlays` callback already passes the full `OverlayContext` your
   layer's `draw` destructures from.

5. **Manually verify** the new overlay against a golden fixture with known
   ground truth (pick one from the `Golden fixture` dropdown, toggle only
   your new layer on via `LayerPanel` to see it in isolation, then toggle
   the others back on to check it composites correctly) — there is no e2e
   framework in this app; visual review against fixtures is the check, per
   `.superpowers/sdd/task-7-report.md`'s executed checklist.
