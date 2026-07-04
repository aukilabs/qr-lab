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
import { makeQrTexture } from "./qrTexture";
import { moduleRegionLocalCornersArray } from "./moduleRegion";
import { projectAllToPixels } from "./projection";
import { flipRowsRgba } from "./rowFlip";
import { applyExposureOffset, applyGaussianBlurCanvas, applyGaussianNoise } from "./camSim";
import { cornerErrors } from "./errorStats";
import { CameraSimControls, type CameraSimValues } from "./CameraSimControls";
import { ErrorPanel, type ErrorSample } from "./ErrorPanel";
import {
  DEFAULT_ECC,
  DEFAULT_PAYLOAD,
  DEFAULT_PHYSICAL_SIZE_M,
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
}

function QrPlane({
  qr,
  physicalSize,
  meshRef,
}: {
  qr: GeneratedQr;
  physicalSize: number;
  meshRef: RefObject<THREE.Mesh | null>;
}) {
  const texture = useMemo(() => makeQrTexture(qr), [qr]);
  useEffect(() => () => texture.dispose(), [texture]);
  return (
    <mesh ref={meshRef} name="qr-plane">
      <planeGeometry args={[physicalSize, physicalSize]} />
      <meshBasicMaterial map={texture} toneMapped={false} />
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
function ScanLoop({ qr, physicalSize, client, payload, camSim, meshRef, onResult }: ScanLoopProps) {
  const { gl, camera } = useThree();
  const targetRef = useRef<THREE.WebGLRenderTarget | null>(null);
  const lastScanAtRef = useRef(0);
  const scanningRef = useRef(false);
  const noiseSeedRef = useRef(0x9e3779b9);

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

    const bottomUp = new Uint8Array(resolution * resolution * 4);
    gl.readRenderTargetPixels(target, 0, 0, resolution, resolution, bottomUp);

    // Bottom-up -> top-down (see this file's module doc + rowFlip.ts).
    let rgba = flipRowsRgba(new Uint8ClampedArray(bottomUp), resolution, resolution);
    if (noiseSigma > 0) {
      // Fresh seed per tick (not truly random — deterministic advance of
      // a counter) so consecutive frames don't dither on identical noise,
      // while a single tick's `applyGaussianNoise` call stays pure/
      // reproducible given its seed.
      noiseSeedRef.current = (noiseSeedRef.current + 0x6d2b79f5) >>> 0;
      rgba = applyGaussianNoise(rgba, noiseSigma, noiseSeedRef.current);
    }
    if (exposureOffset !== 0) rgba = applyExposureOffset(rgba, exposureOffset);
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
      }
      setTimingsSample(outcome.timings);
      setErrorSample(outcome.errorSample);
      setSampleId((n) => n + 1);
    },
    [overlayRegistry],
  );

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
            {qr && <QrPlane qr={qr} physicalSize={physicalSize} meshRef={meshRef} />}
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
      </aside>
    </div>
  );
}
