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
                       ├─ downscaleRgba (same call, same maxDim, twice:
                       │    once posted to the worker, once for display —
                       │    see App.tsx's module doc comment for why)
                       ▼
                  ScannerClient.scan()  →  scanner/worker.ts (Web Worker)
                       │                     scan_rgba() from qrk-wasm
                       ▼
                  ScanResult { detections: { finders, triplets, codes, timings },
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
                samplegridLayer, bitsLayer, decodedLayer
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
  drift.
- `scanner/client.ts` / `scanner/worker.ts` — main-thread request/response
  wrapper around the Worker; "latest-wins" queueing so video mode can fire
  a scan per presented frame without an unbounded backlog when frames
  arrive faster than `scan_rgba` can process them.
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

## Add an overlay layer (5-step recipe)

Follow this to add a new debug overlay for a future detection stage (Plan 4
Task 6 added `alignment.ts`/`samplegrid.ts`/`bits.ts`/`decoded.ts` for the
decode pipeline's own stages this same way — read one of those, or the
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
   alignmentLayer, samplegridLayer, bitsLayer, decodedLayer]`).
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
