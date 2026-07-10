// Task 6: the debug UI's shell. Wires together everything Tasks 1-5 built
// (scanner client/worker, viewport, overlay registry/layers, timings panel,
// layer panel) with the two pieces this task adds (SourcePanel, the
// image/video source hooks) into one working app.
//
// Scan pipeline (binding decision, see task-6-brief.md): a source change,
// a resolution change, or a new video frame produces a full-resolution RGBA
// readout, which goes to `ScannerClient.scan` (since Plan 5 Task 1 the
// downscale happens in Rust inside `scan_rgba`; the client copies the
// frame up front, so the caller's rgba stays readable).
// While that scan runs, this file downscales the SAME rgba with the SAME
// `downscaleRgba` call (same maxDim) to build the bitmap actually drawn in
// the `Viewport` — guaranteeing the displayed pixels and the overlay
// coordinates (which are in the worker's `scanWidth`/`scanHeight` space)
// line up exactly, without the worker needing to ship the downscaled rgba
// back over postMessage. A scan superseded by a newer one
// (`StaleScanError`) is dropped silently — a fresher frame is already on
// its way, so there's nothing to report.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import "./App.css";
import { useImageSource } from "./media/useImageSource";
import { lumaAt } from "./media/luma";
import { workingScaleFor } from "./media/scaling";
import { useVideoSource, type VideoFrame } from "./media/useVideoSource";
import { alignmentLayer } from "./overlays/layers/alignment";
import { bitsLayer } from "./overlays/layers/bits";
import { decodedLayer } from "./overlays/layers/decoded";
import { evidenceLayer } from "./overlays/layers/evidence";
import { findersLayer } from "./overlays/layers/finders";
import { groundtruthLayer } from "./overlays/layers/groundtruth";
import { refinedLayer } from "./overlays/layers/refined";
import { robustCodesLayer } from "./overlays/layers/robust-codes";
import { samplegridLayer } from "./overlays/layers/samplegrid";
import { tilesLayer } from "./overlays/layers/tiles";
import { tripletsLayer } from "./overlays/layers/triplets";
import { parseGroundTruth, type GroundTruthCode } from "./overlays/groundtruth-types";
import { createRegistry, type OverlayContext } from "./overlays/registry";
import { Scene3D } from "./scene3d/Scene3D";
import { FilmstripPanel } from "./panels/FilmstripPanel";
import { LadderPanel } from "./panels/LadderPanel";
import { LayerPanel } from "./panels/LayerPanel";
import { RobustPanel, type RobustCaptureMode } from "./panels/RobustPanel";
import { VideoControls } from "./panels/VideoControls";
import {
  DEFAULT_RESOLUTION,
  maxDimFor,
  SourcePanel,
  type ResolutionOption,
  type SourceDescriptor,
} from "./panels/SourcePanel";
import { TimingsPanel, type TimingsSample } from "./panels/TimingsPanel";
import { downscaleRgba } from "./scanner/downscale";
import { ScannerClient, StaleScanError, WorkerInitTimeoutError } from "./scanner/client";
import type {
  RobustConfig,
  RobustDetections,
  RobustPresets,
  RobustSnapshot,
} from "./scanner/robust-types";
import type { ScanResult } from "./scanner/types";
import type { ViewTransform } from "./viewport/transform";
import { Viewport } from "./viewport/Viewport";

/** Registered once at module scope (not per-`App`-instance) since it has no
 * lifecycle of its own — layer enable/disable state living here rather than
 * in React state is `registry.ts`'s own design (see its doc comment), and a
 * single shared instance means toggle state survives a dev-mode remount. */
const overlayRegistry = createRegistry([
  tilesLayer,
  findersLayer,
  tripletsLayer,
  groundtruthLayer,
  alignmentLayer,
  samplegridLayer,
  bitsLayer,
  decodedLayer,
  refinedLayer,
  robustCodesLayer,
  evidenceLayer,
]);

interface ScanState {
  /** The pipeline result — classic scan, or in robust mode the UNIFIED
   * detections the robust envelope carries (same shape, assembled in Rust
   * from the ladder's cross-variant union; trace-less). One pipeline: the
   * classic overlays always draw, robust flags just mean more entries. */
  result: ScanResult | null;
  /** Plan 6 ladder result — `null` outside robust mode. */
  robust: RobustDetections | null;
  /** Per-variant ladder thumbnails — robust mode + capture only. */
  snapshots: RobustSnapshot[] | null;
  wallMs: number;
  roundTripMs: number;
  scanWidth: number;
  scanHeight: number;
}

/** The source's own (pre-downscale) dimensions — what "source dims" in the
 * status bar reports, and the numerator side of `workingScaleFor`. */
interface SourceDims {
  width: number;
  height: number;
}

/** A picked/dropped fixture is always a PNG (`SourceDescriptor`'s `fixture`
 * variant is `mediaKind: "image"`), so a `"video"` source is always the
 * `kind: "file"` variant — this narrows without an extra runtime check
 * beyond the `mediaKind` comparison itself. */
function imageInputFor(source: SourceDescriptor | null): File | string | null {
  if (!source || source.mediaKind !== "image") return null;
  return source.kind === "fixture" ? `/fixtures/${source.fixture.png}` : source.file;
}

function videoInputFor(source: SourceDescriptor | null): File | null {
  if (!source || source.mediaKind !== "video") return null;
  return source.file;
}

/** Task 5: the debug UI's two top-level modes — "media" is everything
 * Tasks 3-4 built (image/video source, viewport, fixture-driven ground
 * truth); "scene3d" is the new orbitable 3D scene (`Scene3D.tsx`), the
 * plan's headline debug-UI feature. Both modes share the same
 * `ScannerClient`/worker (one scanner per `App` mount, see the
 * client-lifecycle effect below) and the same `overlayRegistry` (layer
 * enable/disable state — and therefore the "Layers" panel — is shared
 * across modes, since `registry.enabled` lives at module scope
 * independent of which mode is currently drawing from it). */
type Mode = "media" | "scene3d";

export function App() {
  const [mode, setMode] = useState<Mode>("media");
  const clientRef = useRef<ScannerClient | null>(null);
  const [scannerReady, setScannerReady] = useState(false);
  const [initError, setInitError] = useState<string | null>(null);

  const [source, setSource] = useState<SourceDescriptor | null>(null);
  const [resolution, setResolution] = useState<ResolutionOption>(DEFAULT_RESOLUTION);
  const [groundTruth, setGroundTruth] = useState<GroundTruthCode[] | null>(null);

  // Plan 6 robust mode. Off by default — with `robustEnabled` false the
  // scan path below is byte-identical to the pre-Plan-6 behavior. The
  // config starts `null` and is seeded from `robust_presets()`'s
  // robustFast once the fetch lands (the RobustPanel's master toggle stays
  // disabled until then), so preset values are never hardcoded here.
  const [robustEnabled, setRobustEnabled] = useState(false);
  const [robustConfig, setRobustConfig] = useState<RobustConfig | null>(null);
  // Capture policy, not a boolean: capture (the per-variant thumbnail
  // filmstrip) is pure visualization payload, and serializing it across
  // the worker boundary per VIDEO frame was measured at >200ms round
  // trips. "paused" (the default) scans playing frames without capture —
  // pure ladder cost — and re-captures the frame the user pauses on (see
  // the pause-recapture effect below). Detections (and thus every
  // overlay) are identical in all modes: the unified result always rides
  // the envelope.
  const [robustCaptureMode, setRobustCaptureMode] = useState<RobustCaptureMode>("paused");
  const [robustPresets, setRobustPresets] = useState<RobustPresets | null>(null);
  // The most recent scan that DID carry capture — kept so the filmstrip
  // can keep showing the last captured frame (marked stale) while playing
  // frames scan capture-free. Reset on source change.
  const [lastSnapshots, setLastSnapshots] = useState<RobustSnapshot[] | null>(null);
  // Plan 6 session mode: the STATEFUL temporal video path. Off by default;
  // applies ONLY in robust mode + a video source (App gates below — stills
  // and non-robust are byte-identical to before). The two knobs tune the
  // temporal amortization (rung rotation cadence + cross-frame candidate
  // lifetime); defaults mirror the Rust `SessionConfig` defaults (3, 4).
  const [robustSession, setRobustSession] = useState(false);
  const [sessionRotationPeriod, setSessionRotationPeriod] = useState(3);
  const [sessionPoolTtl, setSessionPoolTtl] = useState(4);
  // Read by `runScan` at call time (a state dep would rebuild the scan
  // callback mid-playback); assigned from `videoState.playing` below.
  const playingRef = useRef(false);
  // Whether session routing is live for the frame `runScan` is about to
  // send — assigned during render (below, once `isVideoMode` is known), read
  // synchronously at the top of `runScan`. Lets one shared `runScan` route
  // video frames through the session while still scanning image frames the
  // classic robust way.
  const sessionActiveRef = useRef(false);

  const [scanState, setScanState] = useState<ScanState | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [sampleId, setSampleId] = useState(0);

  const [sourceDims, setSourceDims] = useState<SourceDims | null>(null);
  const [displayBitmap, setDisplayBitmap] = useState<ImageBitmap | null>(null);
  const displayBitmapRef = useRef<ImageBitmap | null>(null);
  // The downscaled rgba behind `displayBitmap` — kept for the status bar's
  // luma-under-cursor readout, which needs pixel data a bitmap can't give
  // back directly. Always the same generation as `displayBitmap`/`scanState`
  // (all three are set together, only after a scan actually completes).
  const displayFrameRef = useRef<{ rgba: Uint8ClampedArray; width: number; height: number } | null>(
    null,
  );

  const [cursorPos, setCursorPos] = useState<[number, number] | null>(null);
  const [layerVersion, setLayerVersion] = useState(0);

  // Bumped on every `handleSourceChange`. `runScan` snapshots this at call
  // time and re-checks it after each await — a scan started against the
  // PREVIOUS source can still be in flight (the worker only serializes
  // `client.scan()` calls, not this file's own downscale/bitmap work) when
  // the user switches sources; without this guard its (now-stale) results
  // would land after the new source's state was reset, flashing the old
  // image/overlays back for a frame before the new source's own scan
  // finishes and overwrites them again.
  const sourceGenerationRef = useRef(0);

  // Worker + client lifecycle: one per `App` mount. `cancelled` guards
  // against a stray resolve/reject from a StrictMode-doubled effect's first
  // (already-disposed) worker landing after the second one has taken over.
  useEffect(() => {
    let cancelled = false;
    const worker = new Worker(new URL("./scanner/worker.ts", import.meta.url), { type: "module" });
    const client = new ScannerClient(worker);
    clientRef.current = client;
    setScannerReady(false);
    setInitError(null);

    client
      .init()
      .then(() => {
        if (!cancelled) setScannerReady(true);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const message =
          err instanceof WorkerInitTimeoutError || err instanceof Error ? err.message : String(err);
        setInitError(message);
      });

    return () => {
      cancelled = true;
      client.dispose();
      if (clientRef.current === client) clientRef.current = null;
    };
  }, []);

  // One-shot presets fetch (worker-cached client-side) once the scanner is
  // up; also seeds the initial robust config from robustFast — but only if
  // the user hasn't already edited it (a StrictMode re-run or slow fetch
  // must not clobber manual flags).
  useEffect(() => {
    if (!scannerReady) return;
    let cancelled = false;
    clientRef.current
      ?.robustPresets()
      .then((presets) => {
        if (cancelled) return;
        setRobustPresets(presets);
        setRobustConfig((prev) => prev ?? presets.robustFast);
      })
      .catch((err: unknown) => {
        if (!cancelled) console.error("failed to fetch robust presets", err);
      });
    return () => {
      cancelled = true;
    };
  }, [scannerReady]);

  // Release the display bitmap's decoder resources on unmount (per-source
  // swaps release the *previous* bitmap inline in `runScan`, below).
  useEffect(() => {
    return () => {
      displayBitmapRef.current?.close();
    };
  }, []);

  // Robust state collapsed to one dependency key for `runScan`: the full
  // config while robust mode is on (any edit re-scans), a constant
  // sentinel while it is off (config churn — e.g. the startup preset
  // seed — must not re-fire the scan effect; the stale closure it leaves
  // behind is unread on the classic path).
  const robustDepsKey = robustEnabled
    ? JSON.stringify([
        robustConfig,
        robustCaptureMode,
        robustSession,
        sessionRotationPeriod,
        sessionPoolTtl,
      ])
    : "off";

  /**
   * Scan one full-resolution rgba frame, and rebuild the display bitmap /
   * status-bar state from the same frame. Runs for both image mode (one
   * call per source/resolution change) and video mode (one call per
   * presented frame — see `handleVideoFrame`).
   *
   * Ordering: the scan request is POSTED first (so the worker starts
   * immediately), but the display downscale + bitmap are built and
   * committed BEFORE the scan result is awaited — first paint doesn't
   * wait on scan latency, and the display path's read of `rgba` is
   * independent of the scan transport. (`ScannerClient.scan` copies the
   * frame up front and never consumes the caller's buffer — see its
   * ownership doc comment — so this ordering is belt-and-braces on top of
   * that, not the only thing keeping `rgba` readable.)
   */
  const runScan = useCallback(
    async (rgba: Uint8ClampedArray, width: number, height: number) => {
      const client = clientRef.current;
      if (!client || !scannerReady) return;
      const generation = sourceGenerationRef.current;

      const maxDim = maxDimFor(resolution);
      const startedAt = performance.now();
      // Wrapped into an always-resolving "settled" shape so the promise
      // can sit unawaited through the display work below without an early
      // rejection being flagged as unhandled during that window. Both
      // branches settle into the SAME `ScanState`-shaped fields so the
      // commit code below stays branch-free.
      // `refine: true` (Plan 5 Task 7 QA fix): media mode never passed this
      // before, so `refined_corners` stayed `null` on every scan — the
      // `refinedLayer` overlay and the timings panel's `refine` row were
      // both permanently dead in this mode (Scene3D already hardcodes
      // `refine: true` the same way; per Task 6's re-baseline the cost is
      // ~0.1-0.4ms/frame, negligible for a dev tool with no UI toggle).
      // Plan 6: robust mode routes through `scanRobust` instead — its
      // envelope's UNIFIED detections (the ladder's cross-variant union,
      // classic shape) feed the same overlays as a trace-less
      // `ScanResult`, so robust flags simply mean more entries drawn.
      const useRobust = robustEnabled && robustConfig !== null;
      // Session routing (Plan 6): `sessionActiveRef` already folds in robust
      // mode + a non-null config + the session toggle + a video source, so
      // when it's set `robustConfig` is non-null and this is a video frame.
      // Session frames scan through the persistent worker session
      // (`scanSessionFrame`) — capture is inherently OFF there (no
      // filmstrip), so `captureNow` doesn't apply. Image frames (and video
      // frames with the session toggle off) keep the classic `scanRobust`
      // path below.
      const useSession = useRobust && sessionActiveRef.current;
      // Capture policy resolved per frame (not per runScan identity):
      // "paused" reads the live playing flag so a playing video scans
      // capture-free while a paused/stepped/still frame captures.
      const captureNow =
        robustCaptureMode === "always" || (robustCaptureMode === "paused" && !playingRef.current);
      // Both the session and the classic-robust paths return a
      // `RobustScanOutcome` (same envelope), so they share one settle
      // mapping; only the request call differs. `snapshots` rides through as
      // `null` for session (capture off) → the filmstrip shows its
      // session-unavailable hint.
      const robustPromise = useSession
        ? client.scanSessionFrame(rgba, width, height, { maxDim, refine: true })
        : useRobust
          ? client.scanRobust(rgba, width, height, {
              maxDim,
              refine: true,
              config: robustConfig,
              capture: captureNow,
            })
          : null;
      const scanSettled = robustPromise
        ? robustPromise.then(
            (outcome) => ({
              ok: true as const,
              outcome: {
                result: { detections: outcome.result.detections, trace: null },
                robust: outcome.result.robust,
                snapshots: outcome.result.snapshots,
                wallMs: outcome.wallMs,
                scanWidth: outcome.scanWidth,
                scanHeight: outcome.scanHeight,
              },
            }),
            (err: unknown) => ({ ok: false as const, err }),
          )
        : client.scan(rgba, width, height, { maxDim, withTrace: true, refine: true }).then(
            (outcome) => ({
              ok: true as const,
              outcome: {
                result: outcome.result as ScanResult | null,
                robust: null,
                snapshots: null,
                wallMs: outcome.wallMs,
                scanWidth: outcome.scanWidth,
                scanHeight: outcome.scanHeight,
              },
            }),
            (err: unknown) => ({ ok: false as const, err }),
          );

      // Display path: same downscale, same maxDim, same source pixels the
      // worker is scanning right now — guarantees the bitmap drawn in the
      // Viewport is pixel-for-pixel the space the scan's coordinates (and
      // every overlay layer) live in.
      const down = downscaleRgba(rgba, width, height, maxDim);
      let bitmap: ImageBitmap | null = null;
      try {
        // `down.rgba` types as `Uint8ClampedArray<ArrayBufferLike>` (TS's
        // default generic per lib.es5.d.ts), but `ImageData`'s constructor
        // wants the narrower `Uint8ClampedArray<ArrayBuffer>` — at runtime
        // it's always plain-`ArrayBuffer`-backed (either `getImageData(...)
        // .data`, from `useImageSource`/`useVideoSource`, or a fresh
        // `new Uint8ClampedArray(n)` from `downscaleRgba`'s resize path;
        // never a `SharedArrayBuffer` view), so this narrowing is safe.
        const rgbaForBitmap = down.rgba as Uint8ClampedArray<ArrayBuffer>;
        bitmap = await createImageBitmap(new ImageData(rgbaForBitmap, down.width, down.height));
      } catch (err) {
        console.error("failed to build display bitmap", err);
      }
      // The source changed while the bitmap was being built (see
      // `sourceGenerationRef`'s doc comment) — this frame belongs to the
      // previous source; don't paint it over the new source's reset state.
      if (sourceGenerationRef.current !== generation) {
        bitmap?.close();
        return;
      }
      displayFrameRef.current = down;
      if (bitmap) {
        // Defer closing the previous bitmap until after paint. Closing it
        // synchronously here raced Viewport's rAF: a redraw mid-commit saw
        // a detached ImageBitmap (width 0), skipped drawImage after
        // clearRect, and flashed the canvas black — especially visible
        // under robust mode where scan latency stretches the window
        // between consecutive video frames.
        const prev = displayBitmapRef.current;
        displayBitmapRef.current = bitmap;
        setDisplayBitmap(bitmap);
        if (prev && prev !== bitmap) {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              try {
                prev.close();
              } catch {
                // already closed (source swap, etc.)
              }
            });
          });
        }
      }
      setSourceDims({ width, height });

      const settled = await scanSettled;
      if (sourceGenerationRef.current !== generation) return; // stale — see above
      if (!settled.ok) {
        if (settled.err instanceof StaleScanError) return; // superseded — a fresher frame is already on its way
        setScanError(settled.err instanceof Error ? settled.err.message : String(settled.err));
        return;
      }
      const roundTripMs = performance.now() - startedAt;

      setScanError(null);
      setScanState({
        result: settled.outcome.result,
        robust: settled.outcome.robust,
        snapshots: settled.outcome.snapshots,
        wallMs: settled.outcome.wallMs,
        roundTripMs,
        scanWidth: settled.outcome.scanWidth,
        scanHeight: settled.outcome.scanHeight,
      });
      // Keep the newest CAPTURED filmstrip around across capture-free
      // scans (playing video frames under the "paused" policy) so the
      // strip shows the last captured frame — marked stale — instead of
      // flashing empty during playback.
      if (settled.outcome.snapshots) setLastSnapshots(settled.outcome.snapshots);
      setSampleId((n) => n + 1);
    },
    // The robust deps mean any config/capture/toggle change refreshes
    // `runScan`'s identity, which re-fires the image-mode effect below —
    // a live re-scan on every robust-panel edit, same mechanism resolution
    // changes already use. While robust mode is OFF they collapse to one
    // stable sentinel so preset loading / panel edits don't re-scan (the
    // startup preset seed used to trigger a same-source double scan —
    // the close race Viewport's detached-bitmap guard also covers).
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scannerReady, resolution, robustEnabled, robustDepsKey],
  );

  const isVideoMode = source?.mediaKind === "video";
  const imageInput = imageInputFor(source);
  const videoInput = videoInputFor(source);

  // Session mode is live only for robust + a video source with a loaded
  // config and the toggle on. Fed to `runScan` through a ref (so a mid-
  // playback flip doesn't rebuild the scan callback) and used to gate the
  // configure/reset lifecycle effects and the filmstrip's unavailable hint.
  const sessionActive = robustEnabled && robustConfig !== null && robustSession && isVideoMode;
  sessionActiveRef.current = sessionActive;

  const imageSourceState = useImageSource(imageInput);

  // Mode gating (Plan 5 Task 5 review fix): the app has ONE ScannerClient,
  // and in "scene3d" mode the 3D scene's own per-frame scan loop is its
  // sole intended producer. Without this gate, a video left playing in
  // media mode keeps firing `requestVideoFrameCallback` scans after the
  // switch — two producers then contend for the single latest-wins queue,
  // starving the 3D scene's live error panel (each producer keeps
  // superseding the other's queued request). Video frames are dropped
  // here whenever `mode !== "media"`; belt-and-braces on top of the
  // pause-on-switch effect below (a frame callback already in flight when
  // the mode flips can still land after the pause call).
  const handleVideoFrame = useCallback(
    (frame: VideoFrame) => {
      if (mode !== "media") return; // scene3d owns the scanner — see the gating doc above
      void runScan(frame.rgba, frame.width, frame.height);
    },
    [runScan, mode],
  );
  const videoState = useVideoSource(videoInput, handleVideoFrame);

  // Pause the (hidden, still-mounted) video element on leaving media mode
  // so playback — and with it the rVFC frame-callback stream — actually
  // stops rather than burning decode work into dropped frames. On
  // switching back it stays paused; resuming is the user's call (review
  // decision). `pause` is read through a ref because `useVideoSource`
  // recreates its closures every render — depending on `videoState` here
  // would re-run this effect (and call `pause()`) once per render instead
  // of once per mode change. No pure seam worth unit-testing here (the
  // predicate is a bare `mode !== "media"`); covered by Task 7 manual QA:
  // play a video, switch to 3D Scene (video pauses, error panel updates
  // live), switch back (video still paused).
  const videoPauseRef = useRef(videoState.pause);
  videoPauseRef.current = videoState.pause;
  useEffect(() => {
    if (mode !== "media") videoPauseRef.current();
  }, [mode]);

  // Keep `runScan`'s per-frame capture decision fed with the live playing
  // flag (assignment-during-render, same pattern as the refs above).
  playingRef.current = videoState.playing;

  // Pause-recapture (Plan 6): under the "paused frames only" capture
  // policy, playing frames scan capture-free — so when playback stops, the
  // frame the user is now looking at has no filmstrip/baseline data.
  // Re-deliver that exact frame once (no seek, no play-state change);
  // `runScan` then sees `playing === false` and captures it. Keyed on the
  // play→pause TRANSITION, not on `playing`'s value, so scrubbing while
  // paused (each landed seek already captures) doesn't double-scan.
  const videoRecaptureRef = useRef(videoState.recapture);
  videoRecaptureRef.current = videoState.recapture;
  const prevPlayingRef = useRef(false);
  useEffect(() => {
    const wasPlaying = prevPlayingRef.current;
    prevPlayingRef.current = videoState.playing;
    if (
      wasPlaying &&
      !videoState.playing &&
      mode === "media" &&
      robustEnabled &&
      robustCaptureMode === "paused"
    ) {
      videoRecaptureRef.current();
    }
  }, [videoState.playing, mode, robustEnabled, robustCaptureMode]);

  // Session lifecycle (Plan 6): (re)build the worker's persistent session
  // whenever the robust config or the two temporal knobs change while
  // session mode is active. Rebuilding the session drops its old cross-frame
  // candidate pool — so a config/param/toggle change is an implicit reset,
  // and no separate reset call is needed for those. Declared BEFORE the
  // frame-re-scan effect below so the session-config message reaches the
  // worker ahead of the scan-session-frame that a re-scan posts (both are
  // synchronous posts; effect declaration order is their run order).
  useEffect(() => {
    if (!scannerReady || !sessionActive || !robustConfig) return;
    clientRef.current?.configureSession(robustConfig, sessionRotationPeriod, sessionPoolTtl);
  }, [scannerReady, sessionActive, robustConfig, sessionRotationPeriod, sessionPoolTtl]);

  // Video-mode counterpart of image mode's config-edit live-rescan: video
  // frames are PUSHED (rVFC/seeked), so a robust toggle or panel edit
  // while the video sits paused would otherwise leave the displayed frame
  // scanned under the OLD config until the next play/step/scrub.
  // Re-deliver the current frame whenever the robust scan inputs change
  // and no frame stream is running. (During playback the very next pushed
  // frame picks the new config up anyway.) `robustDepsKey` also covers
  // toggling robust OFF — the sentinel value change re-scans the frame
  // classically. Skipped entirely for image sources (their own effect
  // handles it) and before the first video frame (`recapture` no-ops).
  const videoPlayingNow = videoState.playing;
  useEffect(() => {
    if (mode === "media" && isVideoMode && !videoPlayingNow) {
      videoRecaptureRef.current();
    }
    // Deliberately NOT keyed on `videoPlayingNow`/`isVideoMode` value
    // changes re-triggering a scan — pause transitions are the previous
    // effect's job; this one fires on scan-input changes only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [robustDepsKey, robustEnabled]);

  // Image mode: (re-)scan whenever the decoded source or the resolution
  // changes. `runScan`'s identity already changes with `resolution` (and
  // `scannerReady`), so listing it here covers both "source change" and
  // "resolution change" from the brief without duplicating that logic.
  // Mode-gated like `handleVideoFrame` above — and since `mode` is a dep,
  // switching BACK to media re-fires this effect and re-scans the current
  // image, repopulating the media viewport's overlays after the 3D scene
  // had the scanner to itself.
  useEffect(() => {
    if (mode !== "media") return; // scene3d owns the scanner — see the gating doc above
    if (imageSourceState.loading || imageSourceState.error || !imageSourceState.rgba) return;
    void runScan(imageSourceState.rgba, imageSourceState.width, imageSourceState.height);
  }, [
    mode,
    imageSourceState.rgba,
    imageSourceState.width,
    imageSourceState.height,
    imageSourceState.loading,
    imageSourceState.error,
    runScan,
  ]);

  // Ground truth: fetch + parse a fixture's JSON when it has one; `null`
  // for real/-subdir photos and any dropped file (image or video).
  useEffect(() => {
    let cancelled = false;
    if (source?.kind === "fixture" && source.fixture.json) {
      const jsonPath = source.fixture.json;
      fetch(`/fixtures/${jsonPath}`)
        .then((res) => {
          if (!res.ok) throw new Error(`ground truth: HTTP ${res.status} for ${jsonPath}`);
          return res.json() as Promise<unknown>;
        })
        .then((data) => {
          if (cancelled) return;
          const codes = (data as { codes?: unknown[] }).codes ?? [];
          setGroundTruth(codes.map(parseGroundTruth));
        })
        .catch((err: unknown) => {
          if (cancelled) return;
          console.error("failed to load ground truth", err);
          setGroundTruth(null);
        });
    } else {
      setGroundTruth(null);
    }
    return () => {
      cancelled = true;
    };
  }, [source]);

  const handleSourceChange = (next: SourceDescriptor) => {
    sourceGenerationRef.current += 1;
    // A new scene: drop the session's cross-frame candidate pool so stale
    // candidates from the previous source can't seed the new one (Plan 6).
    // No-op in the worker if no session exists; the configure effect
    // rebuilds it when the new source is a video and session mode is on.
    clientRef.current?.resetSession();
    setSource(next);
    setScanState(null);
    setScanError(null);
    setSourceDims(null);
    setLastSnapshots(null);
    setCursorPos(null);
    displayFrameRef.current = null;
    displayBitmapRef.current?.close();
    displayBitmapRef.current = null;
    setDisplayBitmap(null);
  };

  // Scrubber grab (Plan 6): a click-to-jump or drag on the timeline is a
  // large temporal jump that invalidates the session's cross-frame pool —
  // reset it so the landed frames rebuild fresh. Fires once per grab (the
  // range input's pointerdown covers both click and drag); small ±1-frame
  // steps (the frame buttons / arrow keys) are near-duplicates and left
  // alone. No-op in the worker outside session mode.
  const handleScrubStart = useCallback(() => {
    clientRef.current?.resetSession();
  }, []);

  const handleRescan = () => {
    if (imageSourceState.rgba) {
      void runScan(imageSourceState.rgba, imageSourceState.width, imageSourceState.height);
    }
  };

  // A resolution change re-scans at a different `maxDim` (see the
  // image-mode effect below), so the working `scanWidth`/`scanHeight` for
  // any scan already in flight at the OLD resolution no longer matches
  // what's about to be displayed. Bump the generation exactly like
  // `handleSourceChange` does, so `runScan`'s post-await generation check
  // drops that stale scan instead of letting it land — a mismatched
  // resolution's overlays would otherwise flash onto the new display for
  // one frame (same class of bug `sourceGenerationRef` already guards
  // against for source changes).
  const handleResolutionChange = (next: ResolutionOption) => {
    sourceGenerationRef.current += 1;
    setResolution(next);
  };

  const handleLayerToggle = useCallback(() => setLayerVersion((v) => v + 1), []);

  const workingScale = sourceDims && scanState ? workingScaleFor(sourceDims.width, scanState.scanWidth) : 1;

  const handleOverlays = useCallback(
    (ctx: CanvasRenderingContext2D, view: ViewTransform) => {
      const o: OverlayContext = {
        ctx,
        view,
        scan: scanState?.result ?? null,
        groundTruth,
        imageSize: scanState ? [scanState.scanWidth, scanState.scanHeight] : [0, 0],
        workingScale,
        robust: scanState?.robust ?? null,
      };
      overlayRegistry.drawAll(o);
    },
    // `layerVersion` isn't read in the body above — it's included purely to
    // change this callback's identity when `LayerPanel` toggles a layer, so
    // `Viewport` (which redraws on `overlays` identity change) repaints
    // even though `registry.enabled` mutates in place. See `LayerPanel`'s
    // `onToggle` doc comment.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scanState, groundTruth, workingScale, layerVersion],
  );

  // Robust mode feeds the timings panel `variants[0]` — the baseline
  // pass's per-stage timings, keeping the sparkline alive; the LADDER's
  // whole-frame cost (`robust.total_ns`, all rungs) belongs to LadderPanel
  // instead. Classic mode is unchanged.
  const timingsForSample = scanState
    ? scanState.robust
      ? (scanState.robust.variants[0]?.timings ?? null)
      : (scanState.result?.detections.timings ?? null)
    : null;
  const timingsSample: TimingsSample | null =
    scanState && timingsForSample
      ? {
          timings: timingsForSample,
          wallMs: scanState.wallMs,
          roundTripMs: scanState.roundTripMs,
          // Whole-ladder wall time (all rungs incl. baseline) — 0/absent in
          // classic mode, where the row reads "n/a".
          ladderTotalNs: scanState.robust?.total_ns ?? null,
        }
      : null;

  const cursorLuma = useMemo(() => {
    const frame = displayFrameRef.current;
    if (!cursorPos || !frame) return null;
    return lumaAt(frame.rgba, frame.width, frame.height, cursorPos[0], cursorPos[1]);
    // `displayFrameRef` is a ref (no re-render on write); `scanState`/
    // `displayBitmap` change on exactly the same cadence the ref's contents
    // do, so including one of them as a dep re-derives this after every
    // frame the ref could have changed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cursorPos, scanState, displayBitmap]);

  const rescanDisabled = isVideoMode || imageSourceState.loading || !imageSourceState.rgba;

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark" aria-hidden />
          <span className="brand-title">QRK</span>
          <span className="brand-sub">SCAN · DEBUG</span>
        </div>
        <div className="sidebar-body">
          <div className="mode-tabs">
            <button
              type="button"
              className={mode === "media" ? "mode-tab mode-tab-active" : "mode-tab"}
              onClick={() => setMode("media")}
            >
              Media
            </button>
            <button
              type="button"
              className={mode === "scene3d" ? "mode-tab mode-tab-active" : "mode-tab"}
              onClick={() => setMode("scene3d")}
            >
              3D Scene
            </button>
          </div>

          {mode === "media" && (
            <SourcePanel
              source={source}
              onSourceChange={handleSourceChange}
              resolution={resolution}
              onResolutionChange={handleResolutionChange}
              onRescan={handleRescan}
              rescanDisabled={rescanDisabled}
            />
          )}
          {mode === "media" && (
            <section className="panel-section">
              <h2 className="panel-title">Robust</h2>
              <RobustPanel
                enabled={robustEnabled}
                onEnabledChange={setRobustEnabled}
                config={robustConfig}
                onConfigChange={setRobustConfig}
                captureMode={robustCaptureMode}
                onCaptureModeChange={setRobustCaptureMode}
                presets={robustPresets}
                sessionEnabled={robustSession}
                onSessionEnabledChange={setRobustSession}
                sessionRotationPeriod={sessionRotationPeriod}
                onSessionRotationPeriodChange={setSessionRotationPeriod}
                sessionPoolTtl={sessionPoolTtl}
                onSessionPoolTtlChange={setSessionPoolTtl}
              />
            </section>
          )}
          <section className="panel-section">
            <h2 className="panel-title">Layers</h2>
            <LayerPanel
              registry={overlayRegistry}
              onToggle={handleLayerToggle}
              disabled={
                mode === "media" && robustEnabled
                  ? { tiles: "no tile trace in robust mode (the robust envelope carries no trace)" }
                  : undefined
              }
            />
          </section>
          {mode === "media" && (
            <section className="panel-section">
              <h2 className="panel-title">Timings</h2>
              <TimingsPanel sample={timingsSample} sampleId={sampleId} />
            </section>
          )}
          {mode === "media" && robustEnabled && (
            <section className="panel-section">
              <h2 className="panel-title">Ladder</h2>
              <LadderPanel robust={scanState?.robust ?? null} />
            </section>
          )}
        </div>
      </aside>

      {mode === "media" ? (
        <main className="main">
          {initError && (
            <div className="banner banner-error">Scanner worker failed to start: {initError}</div>
          )}
          {scanError && <div className="banner banner-error">Scan failed: {scanError}</div>}
          {imageSourceState.error && (
            <div className="banner banner-error">Source load failed: {imageSourceState.error}</div>
          )}
          {videoState.error && (
            <div className="banner banner-error">Video error: {videoState.error}</div>
          )}

          <div className="viewport-wrap">
            <Viewport image={displayBitmap} overlays={handleOverlays} onCursorImagePos={setCursorPos} />
            {!scannerReady && !initError && <div className="loading-overlay">Loading scanner…</div>}
          </div>

          {isVideoMode && <VideoControls videoState={videoState} onScrubStart={handleScrubStart} />}

          {robustEnabled && (sessionActive || robustCaptureMode !== "off") && (
            <FilmstripPanel
              snapshots={sessionActive ? null : (scanState?.snapshots ?? lastSnapshots)}
              stale={!sessionActive && !scanState?.snapshots && lastSnapshots !== null}
              sessionMode={sessionActive}
            />
          )}

          <div className="status-bar">
            <span>
              source: {sourceDims ? `${sourceDims.width}×${sourceDims.height}` : "–"}
            </span>
            <span>
              working: {scanState ? `${scanState.scanWidth}×${scanState.scanHeight}` : "–"}
            </span>
            <span>scale: {workingScale.toFixed(3)}</span>
            <span>
              cursor:{" "}
              {cursorPos ? `${Math.round(cursorPos[0])}, ${Math.round(cursorPos[1])}` : "–"}
            </span>
            <span>luma: {cursorLuma ?? "–"}</span>
          </div>
        </main>
      ) : (
        <Scene3D client={clientRef.current} scannerReady={scannerReady} overlayRegistry={overlayRegistry} />
      )}

      {/* Hidden decode surface for video mode — the visible bitmap is the
          downscaled frame drawn in `Viewport`, not this element itself. */}
      <video ref={videoState.videoRef} muted playsInline style={{ display: "none" }} />
    </div>
  );
}
