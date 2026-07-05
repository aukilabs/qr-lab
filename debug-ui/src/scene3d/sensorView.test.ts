import { describe, expect, it } from "vitest";
import { anyKnobActive, sensorViewActive } from "./sensorView";

const DEFAULTS = { blurSigma: 0, noiseSigma: 0, exposureOffset: 0 };

describe("anyKnobActive", () => {
  it("is false at the all-default knob values", () => {
    expect(anyKnobActive(DEFAULTS)).toBe(false);
  });

  it("is true when blur is above 0", () => {
    expect(anyKnobActive({ ...DEFAULTS, blurSigma: 0.1 })).toBe(true);
  });

  it("is true when noise is above 0", () => {
    expect(anyKnobActive({ ...DEFAULTS, noiseSigma: 0.5 })).toBe(true);
  });

  it("is true for a nonzero exposure offset in EITHER direction", () => {
    expect(anyKnobActive({ ...DEFAULTS, exposureOffset: 10 })).toBe(true);
    expect(anyKnobActive({ ...DEFAULTS, exposureOffset: -10 })).toBe(true);
  });
});

describe("sensorViewActive", () => {
  it("mode 'on' forces active regardless of knobs", () => {
    expect(sensorViewActive("on", DEFAULTS)).toBe(true);
  });

  it("mode 'off' forces inactive regardless of knobs", () => {
    expect(sensorViewActive("off", { blurSigma: 3, noiseSigma: 8, exposureOffset: 60 })).toBe(false);
  });

  it("mode 'auto' follows anyKnobActive", () => {
    expect(sensorViewActive("auto", DEFAULTS)).toBe(false);
    expect(sensorViewActive("auto", { ...DEFAULTS, blurSigma: 1 })).toBe(true);
    expect(sensorViewActive("auto", { ...DEFAULTS, exposureOffset: -1 })).toBe(true);
  });
});
