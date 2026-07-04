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

export interface CameraSimValues {
  resolution: RenderResolution;
  blurSigma: number;
  noiseSigma: number;
  exposureOffset: number;
}

export interface CameraSimControlsProps {
  values: CameraSimValues;
  onChange: (values: CameraSimValues) => void;
}

function row(label: string, control: ReactNode, valueLabel: string) {
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12 }}>
      <span style={{ display: "flex", justifyContent: "space-between", color: "#9ca3af" }}>
        <span>{label}</span>
        <span style={{ fontVariantNumeric: "tabular-nums" }}>{valueLabel}</span>
      </span>
      {control}
    </label>
  );
}

export function CameraSimControls({ values, onChange }: CameraSimControlsProps) {
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
    </div>
  );
}
