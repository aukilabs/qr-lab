// Mode 1: the 3D orbit debug scene (Plan 5 Task 5) — the debug UI's
// headline feature. An orbitable react-three-fiber scene renders a plane
// textured with a real, decodable, generated QR code; every ~100ms (or
// when the render-resolution knob changes) the scene's own camera view
// is rendered to an offscreen target, read back as rgba, run through the
// camera-sim knobs (blur/noise/exposure — all documented approximations,
// see `camSim.ts`), and handed to the SAME `ScannerClient` the media mode
// uses (with `refine: true`). The plane's module-region corners are also
// projected through the SAME camera each frame (`moduleRegion.ts` +
// `projection.ts`) as an analytic ground truth, so the error panel can
// show live |refined - truth| accuracy as you orbit.
//
// Pixel-space simplification: every scan here runs with `maxDim: 0` (no
// downscale) at exactly the readback's own resolution, so `source_scale`
// is always `1.0` and `refined_corners` already live in the SAME pixel
// space the ground-truth projection targets — no working/source scale
// conversion needed anywhere in this file (contrast with the media
// mode's fixture ground truth, which the `groundtruth`/`refined` overlay
// layers scale via `OverlayContext.workingScale`).
//
// Readback approach: renders to a `THREE.WebGLRenderTarget` (rather than
// `preserveDrawingBuffer` on the default canvas) — decouples the scan
// resolution from the on-screen display resolution/dpr, and avoids the
// performance cost `preserveDrawingBuffer: true` imposes on EVERY frame
// (it forces the browser to keep a full extra backbuffer copy around
// whether or not this file's throttled scan loop actually reads it that
// frame); the tradeoff is one extra `gl.render` call per scan tick.
// `readRenderTargetPixels` returns rows BOTTOM-UP (same as
// `readPixels` — see `rowFlip.ts`'s doc) — flipped before anything else
// touches the buffer.
import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import * as THREE from "three";
import { GenerateQrError, generateQr, type GeneratedQr } from "../scanner/qrgen";
import { ScannerClient, StaleScanError } from "../scanner/client";
import type { ScanResult } from "../scanner/types";
import type { GroundTruthCode } from "../overlays/groundtruth-types";
import type { OverlayRegistry } from "../overlays/registry";
import { TimingsPanel, type TimingsSample } from "../panels/TimingsPanel";
import { makeQrTexture, type QrColorOptions } from "./qrTexture";
import { moduleRegionLocalCornersArray, moduleRegionPhysicalSize } from "./moduleRegion";
import { projectAllToPixels } from "./projection";
import { flipRowsRgba } from "./rowFlip";
import { applyExposureOffset, applyGaussianBlurCanvas, applyGaussianNoise } from "./camSim";
import { cornerErrors } from "./errorStats";
import { CameraSimControls, type CameraSimValues } from "./CameraSimControls";
import { ErrorPanel, type ErrorSample } from "./ErrorPanel";
import { QrAppearanceControls, FixtureSaveControls, type QrAppearanceValues } from "./SceneControls";
import { BackgroundPlane } from "./BackgroundPlane";
import { computeCameraStats, type CameraStats } from "./cameraStats";
import { drawHud, formatHudLines } from "./hud";
import { expectedInverted } from "./colorUtils";
import { intrinsicsFromFov } from "./intrinsics";
import {
  buildFixtureMeta,
  defaultFixtureName,
  eccLetterFromIndex,
  opaquePlateFromAlpha,
  saveSceneFixture,
  versionFromDim,
} from "./fixtureExport";
import {
  DEFAULT_ECC,
  DEFAULT_PAYLOAD,
  DEFAULT_PHYSICAL_SIZE_M,
  DEFAULT_QR_BG_ALPHA,
  DEFAULT_QR_BG_COLOR,
  DEFAULT_QR_INK_COLOR,
  DEFAULT_RENDER_RESOLUTION,
  DEFAULT_VERSION,
  QUIET_MODULES,
  SCAN_THROTTLE_MS,
} from "./consts";

export interface Scene3DProps {
  client: ScannerClient | null;
  scannerReady: boolean;
  overlayRegistry: OverlayRegistry;
}

interface FrameOutcome {
  scan: ScanResult;
  truth: GroundTruthCode[];
  errorSample: ErrorSample | null;
  timings: TimingsSample;
  /** This tick's camera/plane geometry (Plan 5d HUD + fixture export) —
   * computed once per throttled scan tick (see `cameraStats.ts`'s doc on
   * why per-frame would be wasteful for a display-only stat). */
  cameraStats: CameraStats;
  /** The clean POST-camSim readback rgba this tick scanned — exactly what
   * the scanner last saw (no HUD/overlays baked in), a fresh snapshot
   * (not a view over a reused scratch buffer — see `ScanScratch`'s doc)
   * so it stays valid after this tick's scratch buffers are mutated
   * again. Captured every tick (not just on decode success) so "Save as
   * fixture" always has the latest frame available. */
  capturedRgba: Uint8ClampedArray;
  /** = camSim.resolution at capture time (both the readback's width and
   * height, always square — see this file's module doc). */
  resolution: number;
  /** The three.js camera's vertical fov (degrees) at capture time — feeds
   * `intrinsicsFromFov` for the fixture-export `camera` block. */
  fovDeg: number;
}

function QrPlane({
  qr,
  physicalSize,
  colors,
  meshRef,
}: {
  qr: GeneratedQr;
  physicalSize: number;
  colors: QrColorOptions;
  meshRef: RefObject<THREE.Mesh | null>;
}) {
  const texture = useMemo(() => makeQrTexture(qr, QUIET_MODULES, undefined, colors), [qr, colors]);
  useEffect(() => () => texture.dispose(), [texture]);
  // `transparent` only needs to be true when the background alpha is <1
  // (see `qrTexture.ts`'s doc: ink is always painted fully opaque, only
  // the "paper" fill carries the alpha slider) — leaving it false at
  // alpha=1 matches the pre-5d opaque-plate rendering path exactly.
  return (
    <mesh ref={meshRef} name="qr-plane">
      <planeGeometry args={[physicalSize, physicalSize]} />
      <meshBasicMaterial map={texture} toneMapped={false} transparent={colors.bgAlpha < 1} />
    </mesh>
  );
}

interface ScanLoopProps {
  qr: GeneratedQr;
  physicalSize: number;
  client: ScannerClient;
  payload: string;
  camSim: CameraSimValues;
  meshRef: RefObject<THREE.Mesh | null>;
  onResult: (outcome: FrameOutcome) => void;
}

/** Lives inside the `<Canvas>` (needs `useThree`/`useFrame`) — the actual
 * per-tick render-to-target -> readback -> camera-sim -> scan pipeline.
 * Renders nothing itself (`return null`); all of its work is side
 * effects reported via `onResult`. */
/** Per-tick scratch buffers, reused across scan ticks and reallocated
 * only when the render resolution changes (Plan 5 Task 5 review fix —
 * the naive version allocated ~5 fresh `resolution² * 4`-byte buffers per
 * 100ms tick). `readback` receives `readRenderTargetPixels`' bottom-up
 * bytes; `frame` holds the top-down flip and is then mutated IN PLACE by
 * the noise/exposure passes (safe: both are strictly per-pixel — see
 * `camSim.ts`'s `resolveOut` doc). Reuse across ticks is safe w.r.t. the
 * scanner because `ScannerClient.scan` copies the frame up front (see its
 * ownership doc comment) — a later tick overwriting `frame` can't touch
 * an in-flight scan's snapshot. The blur pass (when active) still
 * allocates: it round-trips through a 2D canvas and `getImageData`
 * always returns a fresh buffer — unavoidable, and it only costs when
 * the blur knob is actually nonzero. */
interface ScanScratch {
  res: number;
  readback: Uint8Array;
  frame: Uint8ClampedArray;
}

function ScanLoop({ qr, physicalSize, client, payload, camSim, meshRef, onResult }: ScanLoopProps) {
  const { gl, camera } = useThree();
  const targetRef = useRef<THREE.WebGLRenderTarget | null>(null);
  const lastScanAtRef = useRef(0);
  const scanningRef = useRef(false);
  const noiseSeedRef = useRef(0x9e3779b9);
  const scratchRef = useRef<ScanScratch | null>(null);

  useEffect(() => {
    const target = new THREE.WebGLRenderTarget(camSim.resolution, camSim.resolution, {
      format: THREE.RGBAFormat,
      type: THREE.UnsignedByteType,
    });
    targetRef.current = target;
    return () => {
      target.dispose();
      if (targetRef.current === target) targetRef.current = null;
    };
  }, [camSim.resolution]);

  useFrame((state) => {
    const target = targetRef.current;
    const mesh = meshRef.current;
    if (!target || !mesh) return;
    const now = performance.now();
    if (scanningRef.current || now - lastScanAtRef.current < SCAN_THROTTLE_MS) return;
    lastScanAtRef.current = now;
    scanningRef.current = true;

    const { resolution, blurSigma, noiseSigma, exposureOffset } = camSim;
    const wallStart = performance.now();

    const prevTarget = gl.getRenderTarget();
    gl.setRenderTarget(target);
    gl.render(state.scene, camera);
    gl.setRenderTarget(prevTarget);

    // Reused scratch buffers — realloc only on resolution change (see
    // `ScanScratch`'s doc for the full reuse-safety argument).
    let scratch = scratchRef.current;
    if (!scratch || scratch.res !== resolution) {
      scratch = {
        res: resolution,
        readback: new Uint8Array(resolution * resolution * 4),
        frame: new Uint8ClampedArray(resolution * resolution * 4),
      };
      scratchRef.current = scratch;
    }

    gl.readRenderTargetPixels(target, 0, 0, resolution, resolution, scratch.readback);
    // A clamped VIEW over the readback's own storage (no copy) — just a
    // type adapter, since readRenderTargetPixels wants Uint8Array and the
    // image pipeline speaks Uint8ClampedArray.
    const bottomUp = new Uint8ClampedArray(
      scratch.readback.buffer,
      scratch.readback.byteOffset,
      scratch.readback.length,
    );

    // Camera-sim pipeline order: row-flip -> noise -> exposure -> blur.
    // Noise and exposure run BEFORE blur deliberately-ish (the blur then
    // smooths the just-added noise, muting the noise knob's visible
    // effect at high blur sigmas) — a physical camera actually noises
    // AFTER optical blur, so this ordering slightly understates noise
    // under blur; acceptable for a sim whose passes are all documented
    // approximations anyway (see camSim.ts), just don't be surprised
    // that maxing blur visually "eats" the noise slider.
    // Bottom-up -> top-down flip first (see this file's module doc +
    // rowFlip.ts); then noise/exposure mutate `scratch.frame` in place.
    let rgba = flipRowsRgba(bottomUp, resolution, resolution, scratch.frame);
    if (noiseSigma > 0) {
      // Fresh seed per tick (not truly random — deterministic advance of
      // a counter) so consecutive frames don't dither on identical noise,
      // while a single tick's `applyGaussianNoise` call stays pure/
      // reproducible given its seed.
      noiseSeedRef.current = (noiseSeedRef.current + 0x6d2b79f5) >>> 0;
      rgba = applyGaussianNoise(rgba, noiseSigma, noiseSeedRef.current, rgba);
    }
    if (exposureOffset !== 0) rgba = applyExposureOffset(rgba, exposureOffset, rgba);
    if (blurSigma > 0) {
      rgba = applyGaussianBlurCanvas(rgba, resolution, resolution, blurSigma, (w, h) => {
        const c = document.createElement("canvas");
        c.width = w;
        c.height = h;
        return c;
      });
    }

    // Ground truth: project the plane's module-region corners through
    // THIS camera into the readback's own resolution x resolution pixel
    // space — see this file's module doc on why no scale conversion is
    // needed here.
    mesh.updateMatrixWorld(true);
    camera.updateMatrixWorld(true);
    const cameraStats = computeCameraStats(camera, mesh.matrixWorld);
    const localCorners = moduleRegionLocalCornersArray(qr.dim, QUIET_MODULES, physicalSize);
    const worldCorners = localCorners.map((c) => new THREE.Vector3(c[0], c[1], c[2]).applyMatrix4(mesh.matrixWorld));
    const truthPxRaw = projectAllToPixels(worldCorners, camera, resolution, resolution);
    const truthPx: [
      [number, number],
      [number, number],
      [number, number],
      [number, number],
    ] = [truthPxRaw[0]!, truthPxRaw[1]!, truthPxRaw[2]!, truthPxRaw[3]!];

    const truth: GroundTruthCode[] = [
      {
        corners_px: truthPx,
        version: qr.dim,
        module_size_px: resolution / (qr.dim + 2 * QUIET_MODULES),
        inverted: false,
        opaque_plate: false,
        payload,
      },
    ];

    // Fresh snapshot (not a view over `scratch`/the blur pass's own
    // buffer) so it survives later ticks mutating those in place — see
    // `FrameOutcome.capturedRgba`'s doc.
    const capturedRgba = rgba.slice();
    const fovDeg = (camera as THREE.PerspectiveCamera).fov ?? 50;

    client
      .scan(rgba, resolution, resolution, { maxDim: 0, withTrace: true, refine: true })
      .then((outcome) => {
        const roundTripMs = performance.now() - wallStart;
        const decoded = outcome.result.detections.codes.find((c) => c.payload === payload);
        const errorSample: ErrorSample | null = decoded?.refined_corners
          ? { corners: cornerErrors(decoded.refined_corners, truthPx) }
          : null;
        onResult({
          scan: outcome.result,
          truth,
          errorSample,
          timings: { timings: outcome.result.detections.timings, wallMs: outcome.wallMs, roundTripMs },
          cameraStats,
          capturedRgba,
          resolution,
          fovDeg,
        });
      })
      .catch((err: unknown) => {
        if (!(err instanceof StaleScanError)) {
          console.error("scene3d: scan failed", err);
        }
      })
      .finally(() => {
        scanningRef.current = false;
      });
  });

  return null;
}

export function Scene3D({ client, scannerReady, overlayRegistry }: Scene3DProps) {
  // Configurable QR generation (task brief: "configurable payload/version
  // + physical size"). `payloadInput` is the live text-field value;
  // `payload` is what actually feeds `generateQr` (debounced-by-blur/
  // Enter via `onBlur`/`onKeyDown` below), so a code isn't regenerated on
  // every single keystroke.
  const [payloadInput, setPayloadInput] = useState(DEFAULT_PAYLOAD);
  const [payload, setPayload] = useState(DEFAULT_PAYLOAD);
  const [version, setVersion] = useState(DEFAULT_VERSION);
  const [ecc, setEcc] = useState(DEFAULT_ECC);
  const [physicalSize, setPhysicalSize] = useState(DEFAULT_PHYSICAL_SIZE_M);
  const [camSim, setCamSim] = useState<CameraSimValues>({
    resolution: DEFAULT_RENDER_RESOLUTION,
    blurSigma: 0,
    noiseSigma: 0,
    exposureOffset: 0,
  });

  const [qr, setQr] = useState<GeneratedQr | null>(null);
  const [qrError, setQrError] = useState<string | null>(null);

  // Plan 5d: QR appearance (ink/background colors + background alpha) +
  // scene-background image + fixture-export state.
  const [qrColors, setQrColors] = useState<QrAppearanceValues>({
    inkColor: DEFAULT_QR_INK_COLOR,
    bgColor: DEFAULT_QR_BG_COLOR,
    bgAlpha: DEFAULT_QR_BG_ALPHA,
  });

  const [bgImageFile, setBgImageFile] = useState<File | null>(null);
  const [bgImageUrl, setBgImageUrl] = useState<string | null>(null);
  // Object URL lifecycle: create one per picked file, revoke the PREVIOUS
  // url whenever the file changes (including to `null`, i.e. "cleared")
  // and on unmount — the standard `URL.createObjectURL` cleanup pattern
  // (same shape as every other effect-owned resource in this file, e.g.
  // `QrPlane`'s texture disposal).
  useEffect(() => {
    if (!bgImageFile) {
      setBgImageUrl(null);
      return;
    }
    const url = URL.createObjectURL(bgImageFile);
    setBgImageUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [bgImageFile]);

  // Fixture-name text field: defaults to `scene_<payload-slug>` and
  // stays in sync with the payload UNTIL the user manually edits it (then
  // it's fully theirs — no more auto-following `payload`).
  const [fixtureName, setFixtureName] = useState(() => defaultFixtureName(DEFAULT_PAYLOAD));
  const fixtureNameTouchedRef = useRef(false);
  useEffect(() => {
    if (!fixtureNameTouchedRef.current) setFixtureName(defaultFixtureName(payload));
  }, [payload]);
  const handleFixtureNameChange = useCallback((name: string) => {
    fixtureNameTouchedRef.current = true;
    setFixtureName(name);
  }, []);

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [cameraStats, setCameraStats] = useState<CameraStats | null>(null);
  /** Latest tick's captured frame — everything "Save as fixture" needs
   * that isn't already plain component state, kept in a ref (not React
   * state) since it's write-often/read-rarely (only on the Save button
   * click) and holds a fairly large typed array we don't want to trigger
   * re-renders by replacing every ~100ms tick. */
  const lastFrameRef = useRef<{
    rgba: Uint8ClampedArray;
    resolution: number;
    cameraStats: CameraStats;
    truth: GroundTruthCode;
    fovDeg: number;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    generateQr(payload, version, ecc)
      .then((g) => {
        if (cancelled) return;
        setQr(g);
        setQrError(null);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setQr(null);
        setQrError(err instanceof GenerateQrError ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [payload, version, ecc]);

  const meshRef = useRef<THREE.Mesh | null>(null);
  const overlayCanvasRef = useRef<HTMLCanvasElement | null>(null);

  // Force `.scene3d-canvas-wrap` to an EXACT square in px, sized to fit
  // inside `.scene3d-canvas-area`. Pure CSS (`aspect-ratio: 1/1` +
  // `height: 100%` + `max-width: 100%`) does NOT reliably produce a
  // square here: in a flex row container, an explicit `height: 100%`
  // wins over `aspect-ratio`'s derived width whenever `max-width: 100%`
  // then clamps that derived width down — verified empirically (a
  // portrait-shaped canvas area produced a non-square 566x1260 wrap,
  // which stretched the render vs. the scan loop's SQUARE offscreen
  // target and broke triplet grouping entirely, same class of bug this
  // whole square-container requirement exists to prevent — see the
  // canvas-area doc comment below). Measuring the container directly and
  // setting an explicit `width`/`height` in px sidesteps the whole CSS
  // ambiguity.
  const canvasAreaRef = useRef<HTMLDivElement | null>(null);
  const [squareSize, setSquareSize] = useState(600);

  useEffect(() => {
    const area = canvasAreaRef.current;
    if (!area) return;
    const update = () => {
      const rect = area.getBoundingClientRect();
      const style = getComputedStyle(area);
      const paddingX = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
      const paddingY = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);
      const size = Math.max(64, Math.min(rect.width - paddingX, rect.height - paddingY));
      setSquareSize(size);
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(area);
    return () => ro.disconnect();
  }, []);

  // The overlay canvas's own pixel buffer stays sized to the scan's
  // resolution (not the container's on-screen CSS size) — both it and
  // the WebGL `<Canvas>` are CSS-stretched to fill the SAME container
  // (see `scene3d-overlay`'s styling), so drawing at 1:1 scan-pixel
  // coordinates lines up with what's on screen without a separate
  // `ViewTransform` scale factor.
  useEffect(() => {
    const canvas = overlayCanvasRef.current;
    if (!canvas) return;
    canvas.width = camSim.resolution;
    canvas.height = camSim.resolution;
  }, [camSim.resolution]);

  const [timingsSample, setTimingsSample] = useState<TimingsSample | null>(null);
  const [sampleId, setSampleId] = useState(0);
  const [errorSample, setErrorSample] = useState<ErrorSample | null>(null);

  const handleResult = useCallback(
    (outcome: FrameOutcome) => {
      const canvas = overlayCanvasRef.current;
      const ctx = canvas?.getContext("2d");
      if (ctx && canvas) {
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        overlayRegistry.drawAll({
          ctx,
          view: { scale: 1, tx: 0, ty: 0 },
          scan: outcome.scan,
          groundTruth: outcome.truth,
          imageSize: [canvas.width, canvas.height],
          workingScale: 1,
        });
        // HUD last, on the SAME overlay canvas — never the offscreen
        // readback target (see `hud.ts`'s module doc). Uses this tick's
        // own camSim snapshot (the `FrameOutcome`'s `timings`/`truth`
        // already close over ScanLoop's per-tick values the same way;
        // camSim values themselves come straight from this component's
        // own state below since they don't change mid-tick).
        drawHud(ctx, formatHudLines(camSim, outcome.cameraStats));
      }
      setTimingsSample(outcome.timings);
      setErrorSample(outcome.errorSample);
      setCameraStats(outcome.cameraStats);
      setSampleId((n) => n + 1);
      lastFrameRef.current = {
        rgba: outcome.capturedRgba,
        resolution: outcome.resolution,
        cameraStats: outcome.cameraStats,
        truth: outcome.truth[0]!,
        fovDeg: outcome.fovDeg,
      };
    },
    [overlayRegistry, camSim],
  );

  // "Save as fixture" (feature 6): build the generator-schema JSON from
  // the LATEST captured tick (`lastFrameRef`, not live component state
  // for anything geometry-derived — the truth/camera-stats must match the
  // SAME frame as the captured rgba, not whatever the camera has moved to
  // since) plus this component's own appearance/payload/ecc/physicalSize
  // state (which don't need per-frame freshness — they're user-set knobs,
  // not derived-per-tick values), then triggers the three downloads.
  const handleSaveFixture = useCallback(async () => {
    const frame = lastFrameRef.current;
    if (!frame || !qr) {
      setSaveError("no captured frame yet — let the scene render at least one tick first");
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      const contrast = expectedInverted(qrColors.inkColor, qrColors.bgColor);
      const camera = intrinsicsFromFov(frame.fovDeg, frame.resolution, frame.resolution);
      // physical_size_m is the MODULE-REGION-ONLY size — see
      // `moduleRegion.ts`'s `moduleRegionPhysicalSize` doc for why this
      // differs from the scene's own (full-plane) `physicalSize` state.
      const physicalSizeM = moduleRegionPhysicalSize(qr.dim, QUIET_MODULES, physicalSize);
      const meta = buildFixtureMeta({
        name: fixtureName,
        width: frame.resolution,
        height: frame.resolution,
        camera,
        blurSigma: camSim.blurSigma,
        noiseSigma: camSim.noiseSigma,
        exposureOffset: camSim.exposureOffset,
        code: {
          payload,
          version: versionFromDim(qr.dim),
          eccLetter: eccLetterFromIndex(ecc),
          physicalSizeM,
          distanceM: frame.cameraStats.distanceM,
          tiltDeg: frame.cameraStats.incidenceDeg,
          moduleSizePx: frame.truth.module_size_px,
          cornersPx: frame.truth.corners_px,
          inverted: contrast.inverted,
          opaquePlate: opaquePlateFromAlpha(qrColors.bgAlpha),
        },
      });
      await saveSceneFixture({ name: fixtureName, rgba: frame.rgba, meta }, frame.resolution, frame.resolution);
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }, [qr, payload, ecc, physicalSize, camSim, qrColors, fixtureName]);

  return (
    <div className="scene3d">
      {/* `scene3d-canvas-area` centers a FORCED-SQUARE `scene3d-canvas-wrap`
          (CSS `aspect-ratio: 1/1`). This matters beyond looks: `ScanLoop`
          renders into a square `WebGLRenderTarget` (`camSim.resolution x
          camSim.resolution`), but R3F sets `camera.aspect` from the ON-
          SCREEN container's own aspect ratio — if that container weren't
          square, the camera's projection matrix would assume a different
          (non-1:1) aspect than the square offscreen target actually has,
          stretching the rendered geometry (verified empirically: a
          non-square container broke triplet grouping outright — finder
          spacing came out scaled unevenly on the two axes). Forcing the
          on-screen container square keeps `camera.aspect === 1`, matching
          every resolution option. */}
      <div className="scene3d-canvas-area" ref={canvasAreaRef}>
        <div className="scene3d-canvas-wrap" style={{ width: squareSize, height: squareSize }}>
          {qrError && <div className="banner banner-error">QR generation failed: {qrError}</div>}
          <Canvas camera={{ fov: 50, near: 0.01, far: 100, position: [0, 0, physicalSize * 2.5] }}>
            <color attach="background" args={["#05070d"]} />
            <gridHelper
              args={[physicalSize * 8, 20, "#374151", "#1f2937"]}
              position={[0, -physicalSize * 1.2, 0]}
            />
            <BackgroundPlane imageUrl={bgImageUrl} physicalSize={physicalSize} />
            {qr && <QrPlane qr={qr} physicalSize={physicalSize} colors={qrColors} meshRef={meshRef} />}
            <OrbitControls makeDefault minDistance={physicalSize * 0.3} maxDistance={physicalSize * 20} />
            {qr && client && scannerReady && (
              <ScanLoop
                qr={qr}
                physicalSize={physicalSize}
                client={client}
                payload={payload}
                camSim={camSim}
                meshRef={meshRef}
                onResult={handleResult}
              />
            )}
          </Canvas>
          <canvas ref={overlayCanvasRef} className="scene3d-overlay" />
          {!scannerReady && <div className="loading-overlay">Loading scanner…</div>}
        </div>
      </div>

      <aside className="scene3d-sidebar sidebar">
        <section className="panel-section">
          <h2 className="panel-title">Scene</h2>
          <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12 }}>
            <span style={{ color: "#9ca3af" }}>payload</span>
            <input
              type="text"
              value={payloadInput}
              onChange={(e) => setPayloadInput(e.target.value)}
              onBlur={() => payloadInput && setPayload(payloadInput)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && payloadInput) setPayload(payloadInput);
              }}
              style={{ fontFamily: "monospace" }}
            />
          </label>
          <div style={{ display: "flex", gap: 8 }}>
            <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12, flex: 1 }}>
              <span style={{ color: "#9ca3af" }}>version (0=auto)</span>
              <input
                type="number"
                min={0}
                max={40}
                value={version}
                onChange={(e) => setVersion(Math.max(0, Math.min(40, Number(e.target.value) || 0)))}
              />
            </label>
            <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12, flex: 1 }}>
              <span style={{ color: "#9ca3af" }}>ecc</span>
              <select value={ecc} onChange={(e) => setEcc(Number(e.target.value))}>
                <option value={0}>L</option>
                <option value={1}>M</option>
                <option value={2}>Q</option>
                <option value={3}>H</option>
              </select>
            </label>
          </div>
          <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12 }}>
            <span style={{ color: "#9ca3af" }}>physical size (m)</span>
            <input
              type="range"
              min={0.05}
              max={0.5}
              step={0.01}
              value={physicalSize}
              onChange={(e) => setPhysicalSize(Number(e.target.value))}
            />
            <span style={{ fontVariantNumeric: "tabular-nums" }}>{physicalSize.toFixed(2)}m</span>
          </label>
          {qr && (
            <div style={{ fontSize: 11, color: "#6b7280" }}>
              dim: {qr.dim}×{qr.dim} modules
            </div>
          )}
        </section>

        <section className="panel-section">
          <h2 className="panel-title">Appearance</h2>
          <QrAppearanceControls
            values={qrColors}
            onChange={setQrColors}
            onBgImageChange={setBgImageFile}
            hasBgImage={bgImageFile !== null}
          />
        </section>

        <section className="panel-section">
          <h2 className="panel-title">Camera sim</h2>
          <CameraSimControls values={camSim} onChange={setCamSim} />
        </section>

        <section className="panel-section">
          <h2 className="panel-title">Corner error</h2>
          <ErrorPanel sample={errorSample} sampleId={sampleId} />
        </section>

        <section className="panel-section">
          <h2 className="panel-title">Timings</h2>
          <TimingsPanel sample={timingsSample} sampleId={sampleId} />
        </section>

        <section className="panel-section">
          <h2 className="panel-title">Save as fixture</h2>
          <FixtureSaveControls
            name={fixtureName}
            onNameChange={handleFixtureNameChange}
            onSave={handleSaveFixture}
            saving={saving}
            disabled={!qr || cameraStats === null}
            error={saveError}
          />
        </section>
      </aside>
    </div>
  );
}
