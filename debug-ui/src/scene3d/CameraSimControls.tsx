// Camera-simulation knob sliders (Plan 5 Task 5) — render resolution,
// Gaussian blur, sensor noise, exposure offset. Purely presentational
// (controlled inputs, no state of its own) — `Scene3D.tsx` owns the
// actual values and applies them via `camSim.ts`'s pure functions. Each
// knob is a documented approximation of a real camera property; see
// `camSim.ts`'s module doc for the specifics.
import type { ReactNode } from "react";
import {
  BLUR_SIGMA_RANGE,
  EXPOSURE_OFFSET_RANGE,
  NOISE_SIGMA_RANGE,
  RENDER_RESOLUTIONS,
  type RenderResolution,
} from "./consts";
import type { SensorViewMode } from "./sensorView";

export interface CameraSimValues {
  resolution: RenderResolution;
  blurSigma: number;
  noiseSigma: number;
  exposureOffset: number;
  fisheye: boolean;
  fisheyeStrength: number;
  correctLens: boolean;
}

export interface CameraSimControlsProps {
  values: CameraSimValues;
  onChange: (values: CameraSimValues) => void;
  /** Sensor-view display mode (Plan 5d follow-up — see `sensorView.ts`):
   * "auto" shows the processed readback frame over the WebGL render
   * whenever any knob above is non-default; "on"/"off" force it either
   * way. Lives in this panel because it's the visualization OF these
   * knobs' effect. */
  sensorView: SensorViewMode;
  onSensorViewChange: (mode: SensorViewMode) => void;
}

function row(label: string, control: ReactNode, valueLabel: string) {
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12 }}>
      <span style={{ display: "flex", justifyContent: "space-between", color: "var(--text-muted)" }}>
        <span>{label}</span>
        <span style={{ fontVariantNumeric: "tabular-nums" }}>{valueLabel}</span>
      </span>
      {control}
    </label>
  );
}

export function CameraSimControls({
  values,
  onChange,
  sensorView,
  onSensorViewChange,
}: CameraSimControlsProps) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      {row(
        "render resolution",
        <select
          value={values.resolution}
          onChange={(e) =>
            onChange({ ...values, resolution: Number(e.target.value) as RenderResolution })
          }
        >
          {RENDER_RESOLUTIONS.map((r) => (
            <option key={r} value={r}>
              {r}×{r}
            </option>
          ))}
        </select>,
        `${values.resolution}×${values.resolution}`,
      )}
      {row(
        "lens",
        <select value={values.fisheye ? "fisheye" : "pinhole"}
          onChange={(e) => onChange({ ...values, fisheye: e.target.value === "fisheye" })}>
          <option value="pinhole">pinhole</option>
          <option value="fisheye">fisheye (OpenCV)</option>
        </select>,
        values.fisheye ? "fisheye" : "pinhole",
      )}
      {values.fisheye && <>
        {row("extra barrel distortion", <input type="range" min={0} max={1} step={0.05}
          value={values.fisheyeStrength}
          onChange={(e) => onChange({ ...values, fisheyeStrength: Number(e.target.value) })} />,
          `${Math.round(values.fisheyeStrength * 100)}%`)}
        <label style={{ fontSize: 12 }}>
          <input type="checkbox" checked={values.correctLens}
            onChange={(e) => onChange({ ...values, correctLens: e.target.checked })} />
          scan with known intrinsics
        </label>
        <div style={{ fontSize: 11, color: "var(--text-muted)" }}>
          0% is equidistant fisheye. Zoom or pan toward an edge to stress the lens.
          Sensor view stays on; areas outside the rendered view use the scene background.
          {values.correctLens && " Calibrated scans show decoded corners; internal trace overlays are unavailable."}
        </div>
      </>}
      {row(
        "blur sigma (approx.)",
        <input
          type="range"
          min={BLUR_SIGMA_RANGE.min}
          max={BLUR_SIGMA_RANGE.max}
          step={BLUR_SIGMA_RANGE.step}
          value={values.blurSigma}
          onChange={(e) => onChange({ ...values, blurSigma: Number(e.target.value) })}
        />,
        `${values.blurSigma.toFixed(1)}px`,
      )}
      {row(
        "sensor noise (approx.)",
        <input
          type="range"
          min={NOISE_SIGMA_RANGE.min}
          max={NOISE_SIGMA_RANGE.max}
          step={NOISE_SIGMA_RANGE.step}
          value={values.noiseSigma}
          onChange={(e) => onChange({ ...values, noiseSigma: Number(e.target.value) })}
        />,
        `σ=${values.noiseSigma.toFixed(1)}`,
      )}
      {row(
        "exposure offset (approx.)",
        <input
          type="range"
          min={EXPOSURE_OFFSET_RANGE.min}
          max={EXPOSURE_OFFSET_RANGE.max}
          step={EXPOSURE_OFFSET_RANGE.step}
          value={values.exposureOffset}
          onChange={(e) => onChange({ ...values, exposureOffset: Number(e.target.value) })}
        />,
        `${values.exposureOffset > 0 ? "+" : ""}${values.exposureOffset}`,
      )}
      {row(
        "sensor view",
        <select
          value={values.fisheye ? "on" : sensorView}
          disabled={values.fisheye}
          onChange={(e) => onSensorViewChange(e.target.value as SensorViewMode)}
        >
          <option value="auto">auto (when any knob active)</option>
          <option value="on">on</option>
          <option value="off">off</option>
        </select>,
        values.fisheye ? "on (fisheye)" : sensorView,
      )}
    </div>
  );
}
