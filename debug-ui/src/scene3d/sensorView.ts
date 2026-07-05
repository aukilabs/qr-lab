// "Sensor view" activation logic (Plan 5d follow-up): whether the 3D
// scene should display the PROCESSED post-camSim readback frame (the
// exact rgba the scanner ingests) over the pristine WebGL render, so the
// blur/noise/exposure knobs have a visible on-screen effect instead of
// only changing an invisible internal buffer.
//
// Three modes (a select, not a checkbox — "auto" is the interesting
// default and an indeterminate checkbox is more awkward than a 3-option
// select): "auto" activates whenever any camSim knob is non-default,
// "on"/"off" force it regardless.
export type SensorViewMode = "auto" | "on" | "off";

export interface SensorKnobs {
  blurSigma: number;
  noiseSigma: number;
  exposureOffset: number;
}

/** `true` when any camSim knob is away from its no-op default (blur 0,
 * noise 0, exposure 0) — i.e. the readback the scanner sees actually
 * differs from the pristine render. */
export function anyKnobActive(knobs: SensorKnobs): boolean {
  return knobs.blurSigma > 0 || knobs.noiseSigma > 0 || knobs.exposureOffset !== 0;
}

/** Resolve the sensor-view select + current knob values into "should the
 * processed frame be displayed this tick". */
export function sensorViewActive(mode: SensorViewMode, knobs: SensorKnobs): boolean {
  if (mode === "on") return true;
  if (mode === "off") return false;
  return anyKnobActive(knobs);
}
