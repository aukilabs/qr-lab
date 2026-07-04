// Task 6: the debug UI's shell. Wires together everything Tasks 1-5 built
// (scanner client/worker, viewport, overlay registry/layers, timings panel,
// layer panel) with the two pieces this task adds (SourcePanel, the
// image/video source hooks) into one working app.
//
// Scan pipeline (binding decision, see task-6-brief.md): a source change,
// a resolution change, or a new video frame produces a full-resolution RGBA
// readout, which goes to `ScannerClient.scan` (the worker downscales
// internally via `downscaleRgba` and runs `scan_rgba` on the result; the
// client copies the frame up front, so the caller's rgba stays readable).
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
import { findersLayer } from "./overlays/layers/finders";
import { groundtruthLayer } from "./overlays/layers/groundtruth";
import { tilesLayer } from "./overlays/layers/tiles";
import { tripletsLayer } from "./overlays/layers/triplets";
import { parseGroundTruth, type GroundTruthCode } from "./overlays/groundtruth-types";
import { createRegistry, type OverlayContext } from "./overlays/registry";
import { LayerPanel } from "./panels/LayerPanel";
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
import type { ScanResult } from "./scanner/types";
import type { ViewTransform } from "./viewport/transform";
import { Viewport } from "./viewport/Viewport";

/** Registered once at module scope (not per-`App`-instance) since it has no
 * lifecycle of its own — layer enable/disable state living here rather than
 * in React state is `registry.ts`'s own design (see its doc comment), and a
 * single shared instance means toggle state survives a dev-mode remount. */
const overlayRegistry = createRegistry([tilesLayer, findersLayer, tripletsLayer, groundtruthLayer]);

interface ScanState {
  result: ScanResult;
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

export function App() {
  const clientRef = useRef<ScannerClient | null>(null);
  const [scannerReady, setScannerReady] = useState(false);
  const [initError, setInitError] = useState<string | null>(null);

  const [source, setSource] = useState<SourceDescriptor | null>(null);
  const [resolution, setResolution] = useState<ResolutionOption>(DEFAULT_RESOLUTION);
  const [groundTruth, setGroundTruth] = useState<GroundTruthCode[] | null>(null);

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

  // Release the display bitmap's decoder resources on unmount (per-source
  // swaps release the *previous* bitmap inline in `runScan`, below).
  useEffect(() => {
    return () => {
      displayBitmapRef.current?.close();
    };
  }, []);

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
      // rejection being flagged as unhandled during that window.
      const scanSettled = client.scan(rgba, width, height, { maxDim, withTrace: true }).then(
        (outcome) => ({ ok: true as const, outcome }),
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
        displayBitmapRef.current?.close();
        displayBitmapRef.current = bitmap;
        setDisplayBitmap(bitmap);
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
        wallMs: settled.outcome.wallMs,
        roundTripMs,
        scanWidth: settled.outcome.scanWidth,
        scanHeight: settled.outcome.scanHeight,
      });
      setSampleId((n) => n + 1);
    },
    [scannerReady, resolution],
  );

  const isVideoMode = source?.mediaKind === "video";
  const imageInput = imageInputFor(source);
  const videoInput = videoInputFor(source);

  const imageSourceState = useImageSource(imageInput);

  const handleVideoFrame = useCallback(
    (frame: VideoFrame) => {
      void runScan(frame.rgba, frame.width, frame.height);
    },
    [runScan],
  );
  const videoState = useVideoSource(videoInput, handleVideoFrame);

  // Image mode: (re-)scan whenever the decoded source or the resolution
  // changes. `runScan`'s identity already changes with `resolution` (and
  // `scannerReady`), so listing it here covers both "source change" and
  // "resolution change" from the brief without duplicating that logic.
  useEffect(() => {
    if (imageSourceState.loading || imageSourceState.error || !imageSourceState.rgba) return;
    void runScan(imageSourceState.rgba, imageSourceState.width, imageSourceState.height);
  }, [
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
    setSource(next);
    setScanState(null);
    setScanError(null);
    setSourceDims(null);
    setCursorPos(null);
    displayFrameRef.current = null;
    displayBitmapRef.current?.close();
    displayBitmapRef.current = null;
    setDisplayBitmap(null);
  };

  const handleRescan = () => {
    if (imageSourceState.rgba) {
      void runScan(imageSourceState.rgba, imageSourceState.width, imageSourceState.height);
    }
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

  const timingsSample: TimingsSample | null = scanState
    ? {
        timings: scanState.result.detections.timings,
        wallMs: scanState.wallMs,
        roundTripMs: scanState.roundTripMs,
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
        <SourcePanel
          source={source}
          onSourceChange={handleSourceChange}
          resolution={resolution}
          onResolutionChange={setResolution}
          onRescan={handleRescan}
          rescanDisabled={rescanDisabled}
        />
        <section className="panel-section">
          <h2 className="panel-title">Layers</h2>
          <LayerPanel registry={overlayRegistry} onToggle={handleLayerToggle} />
        </section>
        <section className="panel-section">
          <h2 className="panel-title">Timings</h2>
          <TimingsPanel sample={timingsSample} sampleId={sampleId} />
        </section>
      </aside>

      <main className="main">
        {initError && (
          <div className="banner banner-error">Scanner worker failed to start: {initError}</div>
        )}
        {scanError && <div className="banner banner-error">Scan failed: {scanError}</div>}
        {imageSourceState.error && (
          <div className="banner banner-error">Source load failed: {imageSourceState.error}</div>
        )}
        {videoState.error && <div className="banner banner-error">Video error: {videoState.error}</div>}

        <div className="viewport-wrap">
          <Viewport image={displayBitmap} overlays={handleOverlays} onCursorImagePos={setCursorPos} />
          {!scannerReady && !initError && <div className="loading-overlay">Loading scanner…</div>}
        </div>

        {isVideoMode && (
          <div className="video-controls">
            <button type="button" onClick={() => videoState.stepFrame(-1)} disabled={videoState.playing}>
              ◀ frame
            </button>
            <button
              type="button"
              onClick={() => (videoState.playing ? videoState.pause() : videoState.play())}
            >
              {videoState.playing ? "Pause" : "Play"}
            </button>
            <button type="button" onClick={() => videoState.stepFrame(1)} disabled={videoState.playing}>
              frame ▶
            </button>
            <span className="video-frame-counter">
              frame {videoState.frameIndex} · {videoState.currentTime.toFixed(2)}s /{" "}
              {videoState.duration.toFixed(2)}s
            </span>
            {!videoState.supportsFrameCallback && (
              <span className="video-warn">no requestVideoFrameCallback — degraded capture rate</span>
            )}
          </div>
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

      {/* Hidden decode surface for video mode — the visible bitmap is the
          downscaled frame drawn in `Viewport`, not this element itself. */}
      <video ref={videoState.videoRef} muted playsInline style={{ display: "none" }} />
    </div>
  );
}
