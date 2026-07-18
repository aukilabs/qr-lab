# qrk debug UI

A React + Vite app for visually driving the `qr-lab-core` QR scanner (compiled
to WASM via `qr-lab-wasm`) against golden fixtures, real photos, and arbitrary
dropped images/videos — with every detection stage rendered as an
independently togglable overlay and a live per-stage timings panel. This is
a development tool, not a product: plain CSS, no component library, no
router.

## Setup

From the repo root, the WASM package must be built once (and rebuilt any
time `crates/qr-lab-core`/`crates/qr-lab-wasm` change) before the app can run —
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
worker imports `src/wasm/qr_lab_wasm.js` once at startup, so `npm run
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
                       │     scan_rgba() from qr-lab-wasm — luma + the NN
                       │     downscale now happen in Rust (qr_lab_core::scan)
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
                refinedLayer, robustCodesLayer, evidenceLayer
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
  (`crates/qr-lab-wasm/tests/envelope_snapshot.rs`), not from guessing. If
  `WasmResult`'s Rust shape changes, regenerate the snapshot
  (`UPDATE_SNAPSHOT=1 cargo test -p qr-lab-wasm --test envelope_snapshot`) and
  update `parseScanResult` to match — `envelope.test.ts` fails loudly on
  drift. **Gotcha (found by Plan 4 Task 7's live-browser QA, fixed in the
  same task):** the snapshot is JSON text (`serde_json`), which renders a
  Rust `Option::None` as `null` — but the REAL `scan_rgba` binding
  (`serde_wasm_bindgen::to_value`, `qr-lab-wasm/src/lib.rs`) renders `None` as
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
  `panels/TimingsPanel.tsx`, `panels/VideoControls.tsx` — sidebar/transport
  UI; `App.tsx` is the thin shell wiring all of the above together plus the
  video/image mode split.

## Media mode: video scrubber (Task 5e)

`panels/VideoControls.tsx` is the video-mode transport bar (rendered below
the viewport whenever `source.mediaKind === "video"`): the pre-existing
frame-step/play-pause buttons, a frame counter + `mm:ss.d` time labels, and
a timeline scrubber — a plain `<input type="range" class="video-scrub">`
styled full-width, one row below the buttons.

- **Click anywhere on the bar jumps there; dragging scrubs continuously.**
  Both go through the same `onChange` handler (a native range input fires
  `input`/`change` for a click-to-a-position exactly like a drag) — the
  displayed thumb position updates unthrottled (1:1 with the pointer), but
  the actual `useVideoSource.seek()` call (and therefore the `currentTime`
  write that triggers a `seeked`/rVFC capture → scan) is throttled to
  ~10/s via `media/throttle.ts`'s trailing-edge `throttle()`. On
  `pointerup` the throttle is cancelled and a final, unthrottled `seek()`
  fires directly — the last dragged position always lands exactly, never
  stuck behind the throttle window.
- **Pauses on grab, stays paused after release** (`onPointerDown` calls
  `pause()` if playing) — standard scrub UX; resuming is the user's call,
  same review decision Plan 5 Task 5 made for the mode-switch pause.
  Between drag steps the latest-wins `ScannerClient` queue plus the
  seek/rVFC event chain (`useVideoSource`, unchanged by this task) handle
  one scan per landed scrub position.
- **Displayed position while playing is also throttled to ~10Hz**
  (`media/throttle.ts` again, a separate instance) — synced from
  `currentTime`, which itself updates once per presented frame
  (`requestVideoFrameCallback`, i.e. up to the video's own frame rate) so
  the scrubber's own re-renders don't compound that into extra churn on
  top of what `useVideoSource` already does for the frame counter.
- **Keyboard**: with the scrubber focused, `ArrowLeft`/`ArrowRight` call
  the existing `stepFrame(-1|1)` (same ±1/30s step as the dedicated frame
  buttons) instead of the range input's native (browser-default, coarse)
  arrow-key step — `onKeyDown` calls `preventDefault()` so the native step
  never also fires (would otherwise double-move the position).
- **Disabled until `loadedmetadata`**: `duration` starts at `0` (and stays
  `NaN`/non-finite is defensively handled too) until the video decodes its
  first bit of metadata, gating the whole scrub input — no
  divide-by-zero/`NaN` range, no seeking into an unknown duration.
- **Pure logic split out for unit testing** (this repo has no jsdom — DOM/
  hook behavior stays manual-QA'd, see below): `media/videoTime.ts`
  (`formatTime` mm:ss.d incl. the hours-long-source edge where minutes just
  grow past 60 rather than wrapping to `h:mm:ss`; `clampTime`, shared with
  `useVideoSource.stepFrame`/the new `seek()`) and `media/throttle.ts`
  (trailing-edge throttle with `cancel()`/`flush()`, covered by
  `throttle.test.ts` with an injectable clock — no real timers needed for
  the elapsed-time logic, only for the trailing-call scheduling).
- **Manual QA executed for this task** (scripted headless Chrome via raw
  CDP, same `cdp.mjs`-over-`WebSocket` pattern as Plans 3-5's QA — see
  `.superpowers/sdd/p5e-report.md`): dropped a synthesized 3s test video,
  confirmed the scrub input stays disabled until `loadedmetadata`, dragged
  the thumb via dispatched `Input.dispatchMouseEvent` sequences (asserting
  the video's real `currentTime` lands near the drop point and the frame
  counter increments — proof a scan followed each landed position), a
  separate click-to-jump, paused-after-release, and a focused-scrubber
  `ArrowRight` key press landing exactly one `1/30s` step (not a native
  range step) with no double-fire.

## Robust mode (Plan 6)

Media mode's sidebar gains a "Robust" panel whose master **Robust mode**
checkbox (off by default — when off the scan path is byte-identical to the
classic behavior) routes `App.runScan` through
`ScannerClient.scanRobust` → `scan_rgba_robust`, the adaptive escalation
ladder (`qr_lab_core::scan_robust`), instead of the plain `scan_rgba`. Works
in both image and video mode (the shared latest-wins queue covers video —
a robust request and a plain request occupy the SAME slot, so a newer
request of either kind supersedes a queued one of either kind); the 3D
Scene mode ignores robust mode entirely (its scan loop still calls
`client.scan` directly).

- **Config panel** (`panels/RobustPanel.tsx`): a preset select
  (Baseline / Robust fast / Robust full benchmark) populated from
  `robust_presets()` — the authoritative Rust `ScanConfig` constants,
  fetched once through the worker and cached client-side
  (`ScannerClient.robustPresets()`), never hardcoded in JS; selecting a
  preset sets all fields, and any manual edit flips the select to a
  derived (read-only) "custom" entry. Below it: the seven ladder flag
  checkboxes, `max variants/frame` (0 = unlimited), `early exit`, and
  **Capture ladder buffers** — a three-mode select, because capture
  (baseline-detections clone + per-variant thumbnails) is pure
  VISUALIZATION payload whose worker↔main serialization was measured at
  >200ms round trips per VIDEO frame, while the ladder itself runs
  identically in every mode: `off` (never), `paused frames only` (the
  default — still images and paused/stepped/scrubbed video frames
  capture; PLAYING frames scan at pure ladder cost, and pausing
  auto-recaptures the frame you stopped on via
  `useVideoSource.recapture()`, a re-deliver of the current frame with no
  seek and no play-state change, keyed on the play→pause transition), and
  `every frame (slow)` (per-frame filmstrips during playback, when you
  actually want that). The capture decision is made per frame inside
  `runScan` (it reads the live playing flag through a ref), not baked
  into the callback's identity. The master toggle stays disabled until
  the presets fetch lands (the initial config is seeded from
  `robustFast`).
- **Ladder panel** (`panels/LadderPanel.tsx`, sidebar, below Timings):
  one row per `robust.variants` entry in execution order — stage-colored
  dot (`stageColor`), `variantKindLabel`, a time bar proportional to the
  slowest rung's `total_ns`, F/T/C (finders/triplets/codes) counts, and a
  "+N" badge (row subtly highlighted) on rungs whose `new_codes > 0` —
  the rungs that earned their cost. Footer: whole-ladder ms
  (`robust.total_ns`), variants run, and "early exit ✓" / "budget hit"
  badges. The Timings panel itself is fed `variants[0].timings` (the
  baseline pass) in robust mode, keeping its sparkline alive, plus a
  **`ladder total` row** (`TimingsSample.ladderTotalNs` =
  `robust.total_ns`, all rungs) — without it the recovery rungs' cost is
  invisible there ("n/a" in classic mode, like any stage that didn't
  run).
- **Filmstrip** (`panels/FilmstripPanel.tsx`, main area, robust mode +
  capture not `off`): one grayscale `<canvas>` thumbnail per `snapshots`
  entry — the exact (≤320px) buffer the scanner saw at that rung, labeled
  with `variantKindLabel`; click to enlarge (max 480px, stage-color
  border). Under the `paused frames only` policy, playback keeps the LAST
  captured frame's strip visible — dimmed, with a "capture is paused
  during playback" badge — instead of flashing empty (`stale` prop; the
  retained strip lives in `App`'s `lastSnapshots`, reset on source
  change). No snapshots at all renders an "enable capture" hint.
- **Overlay layers** (`overlays/layers/robust-codes.ts` /
  `evidence.ts`, registered after `refinedLayer`): `robust-codes` strokes
  each `robust.codes` quad in its finding stage's color with a variant
  label chip at TL plus refined-corner dots; `evidence` draws a cyan
  crosshair per `robust.triplet_evidence` point. Both consume SOURCE-px
  geometry (`corners_source`/`refined_corners_source`/
  `triplet_evidence`) multiplied by `workingScale` — the same convention
  as `groundtruth.ts`; `robust.codes[i].code.corners` is that code's own
  VARIANT working px and is never drawn. In robust mode the CLASSIC
  layers (finders/triplets/decoded/refined) draw from the `baseline`
  detections when capture is on (wrapped as a trace-less `ScanResult`);
  capture off leaves them dataless (they draw nothing, by design), and
  the `tiles` layer — trace-only, and the robust envelope carries no
  trace — is disabled in the Layers panel with a tooltip while robust
  mode is on.
- **Contract** (`scanner/robust-types.ts`): mirrors
  `scanner/__snapshots__/envelope.robust.shadow_04.json`, the SECOND
  Rust-generated snapshot (`envelope_snapshot.rs`) alongside
  `envelope.near_00.json` — regenerate with the same `UPDATE_SNAPSHOT=1`
  command when `WasmRobustResult` changes; `parseRobustScanResult` is
  throw-on-drift like `parseScanResult` and shares its exported helper
  parsers. The same **null/undefined rule** applies (see the Key modules
  gotcha above): the snapshot renders `Option::None` as `null`, the live
  binding as `undefined` — `baseline`, `snapshots`, and
  `refined_corners_source` accept both, with tests covering both shapes.
  One extra both-shapes case here: `snapshots[i].luma` is a `Uint8Array`
  over the live boundary (serde_bytes) but a `number[]` in JSON — the
  parser accepts both and normalizes to `Uint8Array`.

### Session mode (temporal, video)

The Robust panel's **Session (temporal video)** checkbox (off by default)
routes robust VIDEO scanning through a STATEFUL `WasmScanSession` instead of
the per-frame `scan_rgba_robust`. On real handheld footage the per-frame
ladder re-runs the full detect batch every frame (~32ms); the session
amortizes it across near-duplicate frames — **rung rotation** (each frame
runs a rotating subset of the ladder) plus a **cross-frame candidate pool**
(finders/triplets pooled over a few frames so a code that no single frame
found alone can still decode). Measured on handheld video: **32→22ms/frame
mean (−30%), +13% triplet-evidence frames, zero payload loss.**

- **Two knobs** (both min 1): `rotation period` (default 3) — how many
  frames a full ladder rotation spans; `pool TTL frames` (default 4) — how
  long a pooled candidate survives before it's evicted. They map to
  `qr_lab_core::SessionConfig`'s defaults.
- **Video + robust only.** The toggle and inputs render always but App gates
  their USE to `source.mediaKind === "video"` with robust mode on; still
  images and the classic (non-robust) path are byte-identical to before —
  a still frame in session mode is scanned the plain robust way
  (`scanRobust`), never through the session.
- **Envelope is identical** to `scan_rgba_robust`'s (`WasmRobustResult`,
  parsed by the same `parseRobustScanResult`), so the timings panel, ladder
  panel, and every overlay work unchanged. `snapshots` is always `null` —
  the session path never captures ladder buffers, so the **filmstrip is
  unavailable in session mode** (it shows an explicit hint instead of the
  strip).
- **The session is stateful and must be reset on a scene change.** It is
  reset (cross-frame pool dropped) on: a source change (`handleSourceChange`
  → `client.resetSession()`), a scrubber grab / seek (`VideoControls`'
  `onScrubStart` → reset — a large temporal jump would otherwise seed the
  new scene with stale candidates), and any config/param/toggle change
  (the configure effect rebuilds the worker session, an implicit reset).
  Small ±1-frame steps (frame buttons / arrow keys) are near-duplicates and
  do NOT reset. Because the session is stateful, re-scanning the SAME
  displayed frame twice (e.g. paused, then a param toggle) advances the
  session's frame counter — expected, not idempotent.
- **Wire protocol** (`scanner/worker.ts` / `client.ts`): three new messages
  alongside the stateless ones. `session-config`
  `{ config, rotationPeriod, poolTtlFrames }` and `session-reset` are
  fire-and-forget (`client.configureSession` / `client.resetSession`);
  `scan-session-frame` `{ id, rgba, width, height, maxDim, refine }`
  (`client.scanSessionFrame`) shares the SAME latest-wins slot as
  `scan`/`scan-robust` (a newer request of any kind supersedes) and answers
  with the SAME `scan-result` envelope. The worker holds one module-scoped
  `WasmScanSession`, rebuilt (old one `.free()`d) on each `session-config`;
  a `scan-session-frame` with no session configured returns an `ok:false`
  error rather than crashing the worker.

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
  (`qr-lab-wasm`'s `qr-gen` feature, `scanner/qrgen.ts`) into a
  `CanvasTexture`; `NearestFilter` (mag) for sharp edges,
  `LinearMipMapLinearFilter` (min) to avoid aliasing under keystone.

**QA findings (Plan 5 Task 7), for anyone re-measuring this scene:**

> **Measurement caveat (Plan 5d review fix):** every error number in this
> section was measured while `projection.ts` still mapped the optical
> axis to `width/2` instead of the pixel-centers-at-integers `(width-1)/2`
> the scanner/fixture convention uses — i.e. the analytic ground truth
> carried a systematic `(+0.5, +0.5)`px bias (~0.71px diagonally) vs. the
> refined corners being compared against it. The projection has since
> been corrected, so freshly measured errors run LOWER than the bands
> below (a correction, not a regression); the qualitative findings
> (flat-across-poses profile, TR/BL vs TL/BR asymmetry, clean grazing
> dropout) still hold.

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

## Scene controls (Plan 5d)

The 3D-scene sidebar's "Appearance" panel and "Save as fixture" panel add
scene-background/QR-color customization and a fixture-export button on top
of Plan 5 Task 5's original scene.

- **Scene background image** (`scene3d/BackgroundPlane.tsx`): a file
  picker under "Appearance" textures a large plane (`BACKGROUND_PLANE_SCALE`
  = 4x the QR's own `physicalSize`) positioned slightly behind the QR
  plane (`BACKGROUND_PLANE_Z_OFFSET`, same orientation, avoiding
  z-fighting) — this is a real scene object, not a CSS background, so it
  IS visible in the offscreen readback the scan loop scans and (with the
  QR background alpha slider below 1, see next item) shows through the
  QR's quiet zone/light modules too. No image picked -> the scene's
  existing dark background (unchanged from Plan 5). `Scene3D.tsx` owns
  the picked `File`'s object-URL lifecycle (`URL.createObjectURL`/
  `revokeObjectURL` on change and unmount); `BackgroundPlane` only
  consumes the URL string.
- **QR background/ink colors + background alpha** (`scene3d/qrTexture.ts`'s
  `QrColorOptions`): ink color paints dark modules (always fully opaque);
  background color + alpha paint the quiet zone + light modules (the
  "paper") — alpha `<1` makes the QR's material `transparent` and lets
  the scene background plane above show through, reproducing the
  `trans_`-fixture-scenario look (`opaque_plate: false` once exported).
- **"reads as: normal/inverted" + low-contrast warning**
  (`scene3d/colorUtils.ts`): the code's inverted-polarity status is
  EMERGENT from `luma(ink)` vs `luma(bg)` (same BT.601 fixed-point
  coefficients as `qr_lab_core::luma_from_rgba`) — no separate flag. A
  warning fires when `|Δluma| < CONTRAST_WARN_THRESHOLD` (30, chosen with
  headroom above the Rust detector's actual per-tile `CONTRAST_FLOOR`,
  12) since blur/noise/exposure erode contrast further on top of a user's
  raw color choice. **Alpha awareness (Plan 5d review fix):** with the
  background alpha below 1 the paper the scanner sees is `bgColor`
  composited over whatever sits behind the plane, which can FLIP polarity
  vs. the flat-color prediction (e.g. white ink on white paper at alpha 0
  over the dark scene reads inverted, not zero-contrast) — the indicator
  composites over the scene background COLOR
  (`expectedInvertedComposited`) when no image is picked, and shows
  "depends on background image (measured at export)" when one is (an
  arbitrary image has no single answer). The EXPORTED `inverted` flag
  never trusts this prediction either way — see the fixture-export item
  below.
- **HUD** (`scene3d/hud.ts`, drawn by `Scene3D.tsx`'s `handleResult`): a
  small monospace block, top-left, on the 2D OVERLAY canvas ONLY — blur
  σ / noise σ / exposure offset (the live camSim knobs) plus camera/plane
  geometry (`scene3d/cameraStats.ts`'s `computeCameraStats`: distance to
  the plane center in meters, incidence angle in degrees [0 = head-on, 90
  = grazing], and an approximate in-plane roll), recomputed once per
  scan tick (throttled to `SCAN_THROTTLE_MS`, not every animation frame).
  This NEVER touches the offscreen `WebGLRenderTarget` the scanner reads
  from — see `hud.ts`'s module doc for why that separation is load-bearing
  (contaminating the readback would feed the scanner's own HUD pixels
  back into itself).
- **Sensor view** (`scene3d/sensorView.ts` + `Scene3D.tsx`'s
  `handleResult`): the on-screen WebGL render never reflects the camSim
  knobs (they post-process an invisible internal buffer), so without this
  the sliders appear to do nothing visually. Sensor view paints the
  PROCESSED post-camSim readback frame — the exact rgba the scanner
  ingested that tick — onto the overlay canvas as the base layer, so
  blur/noise/exposure are visibly ON SCREEN. A "sensor view" select in
  the Camera sim panel: **auto** (default — active whenever any knob is
  non-default), **on**, **off**. The HUD gains a `[sensor view]` tag line
  while active, so it's always explicit that you're looking at the
  scanner's input rather than the live render. Alignment is inherent, not
  mapped: the overlay canvas's pixel buffer is exactly the readback's own
  `resolution × resolution` size and every overlay layer already draws at
  1:1 readback px (`view: {scale: 1}`) on that same canvas, so
  `putImageData` at the origin shares the overlays' coordinate space by
  construction (both are CSS-stretched into the same forced-square
  container together; a resolution change mid-flight skips one frame
  rather than paint a mis-scaled one — see the guard in `handleResult`).
  Cadence: the sensor frame updates at SCAN cadence (`SCAN_THROTTLE_MS`,
  ~10fps), not per animation frame — orbiting under sensor view looks
  slightly steppy by design (the scan loop keeps ticking during
  OrbitControls interaction, so it never freezes); an accepted tradeoff
  for a debug tool.
- **Save as fixture** (`scene3d/fixtureExport.ts`): a text field (default
  `scene_<payload-slug>`, editable — stops auto-following the payload once
  you touch it) + button producing three downloads named `<name>.json` /
  `.png` / `.luma`:
  - The captured frame is the CLEAN post-camSim readback rgba (exactly
    what the scanner last saw that tick — blur/noise/exposure applied,
    NO HUD/overlays), snapshotted fresh every tick (`ScanLoop`'s
    `capturedRgba`) so it survives later ticks mutating scratch buffers.
  - `.png`: that rgba through a temporary canvas's `toBlob('image/png')`.
  - `.luma`: `media/luma.ts`'s `lumaBufferFromRgba` — the exact
    `qr_lab_core::luma_from_rgba` fixed-point formula (77/150/29 over 256),
    not an approximation; verified byte-identical against a
    Python/Pillow-derived luma plane of the same PNG in this feature's
    headless QA pass (see Verification below).
  - `.json`: the full `tools/fixtures/generate.py` schema — `camera`
    intrinsics derived via `scene3d/intrinsics.ts`'s `intrinsicsFromFov`
    (same `fy=(h/2)/tan(fovY/2)`, `fx=fy`, `cx=(w-1)/2`, `cy=(h-1)/2`
    convention as `tools/fixtures/camera.py`'s `Intrinsics.default()`);
    `physical_size_m` is the MODULE-REGION-ONLY size (`moduleRegion.ts`'s
    `moduleRegionPhysicalSize`) — NOT the scene's own `physicalSize` state,
    which is the full plane including the quiet zone (verified against
    `tools/fixtures/render.py`'s `_plane_corners_m`, which treats
    `physical_size_m` as the no-quiet-zone module region); `distance_m`/
    `tilt_deg` come straight from that tick's camera stats;
    `module_size_px` is POSE-DERIVED from the actual projected corners
    (`moduleSizeFromCorners` = `|TR−TL|/dim`, exactly `generate.py`'s
    derivation — Plan 5d review fix: an earlier version exported the
    pose-invariant `resolution/(dim+2·quiet)` texture-density constant,
    ~2.3x too large at the default pose); the `inverted` flag is
    MEASURED from the captured frame, not predicted from the color
    pickers (`probeInvertedFromRgba`: sample luma 0.5 modules diagonally
    inside the TL module-region corner [finder ink] vs 0.5 modules
    outside [paper/scene], the same probe geometry as
    `crates/qr-lab-core/tests/fixtures_smoke.rs`; `inverted =
    lumaInside > lumaOutside`) — necessary because with a translucent
    paper the effective background is whatever the scene composites
    behind it, and gates hard-branch on this flag; if the probe's
    contrast is under 30 the export still completes but a warning is
    shown (the measured flag is unreliable — don't commit that fixture
    unchecked); `tilt_azimuth_deg`/`inplane_deg` are recorded as `0` with
    an inline comment — a single incidence angle + approximate roll can't
    losslessly recover which in-plane axis a tilt happened about, so
    these are informational placeholders, not measured values;
    `exposure_offset` is an EXTRA top-level field (no schema slot for it)
    — harmless, every consumer (the Python generator, the Rust `Meta`
    loader) ignores unknown fields.
  - **Caveat for anyone dropping a saved fixture into `fixtures/`:** the
    Rust loader's `common::load_all()` sweeps every fixture file present,
    so a scene-exported fixture WILL be picked up by `decode_gate`/
    `refine_gate` if committed — that's the intent (verify a scene
    capture against the same gates), but its `corners_px` are the
    scene's own analytic projection, which (per Plan 5 Task 5/7's
    findings above) carries the SAME ~1px rendering-chain bias as the
    live error panel, not the ~0.10px precision of a `tools/fixtures/`-
    generated fixture. Use a `scene_` name prefix (the default) so this
    provenance is visible at a glance, and don't expect a scene-exported
    fixture to tighten `refine_gate`'s bound.

**Verification (headless, this feature's own QA pass):** driven via a
`cdp.mjs`-style raw-CDP Node script (real headless-ish Chrome,
`--remote-debugging-port` + `--remote-allow-origins=*`, no puppeteer in
this dependency tree) against a live `vite dev` server — color/alpha
inputs set via the native-setter + real-event bypass (same pattern as the
Plan 5 Task 7 harness), HUD confirmed both visually (screenshot) and by
sampling the overlay canvas's own pixel alpha (a non-transparent block at
the expected position). "Save as fixture" verified end-to-end: `Page.
setDownloadBehavior` routed the three downloads to a scratch directory;
the `.json` parsed and round-tripped every field; the `.luma` file matched
a Python/Pillow-derived luma plane of the `.png` pixel-for-pixel (dense
sample, zero mismatches); and — the strongest check — the saved `.png`
was fed straight into `cargo run --example decode_photo`, which decoded
it successfully (correct version/ecc/payload, refined corners within
~1px of the exported `corners_px`), closing the full loop from "scene
knob state" to "a real fixture the Rust pipeline can read back."

Sensor view was verified with pixel-metric assertions computed IN-PAGE on
the DISPLAY canvas itself (the `.scene3d-overlay` canvas, central region
— not the internal readback, since the point is the on-screen effect):
auto+default-knobs leaves the region ~99% transparent; forcing "on"
paints a fully opaque sensor frame; exposure −40 dropped the displayed
mean luma by 20; noise σ=8 raised mean|horizontal gradient| ×1.38; blur
3px cut it ×0.84; forcing "off" restored transparency with knobs still
active — 7/7 assertions, plus screenshots confirming the `[sensor view]`
HUD tag and that overlays (finder circles, ground-truth quad, decoded
label) sit exactly on the displayed sensor frame. One environment gotcha
for future headless passes: Chrome throttles `requestAnimationFrame` to
zero for fully occluded windows, which freezes the r3f scene and all scan
ticks — call `Page.bringToFront` before driving the 3D scene.

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
   regenerate the snapshot (`UPDATE_SNAPSHOT=1 cargo test -p qr-lab-wasm
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
   alignmentLayer, samplegridLayer, bitsLayer, decodedLayer, refinedLayer,
   robustCodesLayer, evidenceLayer]`).
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
