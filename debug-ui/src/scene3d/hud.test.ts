import { describe, expect, it } from "vitest";
import { formatHudLines } from "./hud";

describe("formatHudLines", () => {
  it("always includes the three camSim lines", () => {
    const lines = formatHudLines({ blurSigma: 0, noiseSigma: 0, exposureOffset: 0 }, null);
    expect(lines).toHaveLength(3);
    expect(lines[0]).toContain("blur");
    expect(lines[1]).toContain("noise");
    expect(lines[2]).toContain("exposure");
  });

  it("formats blur/noise sigma to one decimal place", () => {
    const lines = formatHudLines({ blurSigma: 1.2, noiseSigma: 3, exposureOffset: 0 }, null);
    expect(lines[0]).toContain("1.2");
    expect(lines[1]).toContain("3.0");
  });

  it("prefixes a positive exposure offset with +, leaves negative as-is", () => {
    const positive = formatHudLines({ blurSigma: 0, noiseSigma: 0, exposureOffset: 20 }, null);
    const negative = formatHudLines({ blurSigma: 0, noiseSigma: 0, exposureOffset: -20 }, null);
    expect(positive[2]).toContain("+20");
    expect(negative[2]).toContain("-20");
    expect(negative[2]).not.toContain("+-20");
  });

  it("appends camera-stats lines when stats are provided", () => {
    const lines = formatHudLines(
      { blurSigma: 0, noiseSigma: 0, exposureOffset: 0 },
      { distanceM: 0.456, incidenceDeg: 12.3, inPlaneRollDeg: -5.6 },
    );
    expect(lines).toHaveLength(6);
    expect(lines[3]).toContain("0.456");
    expect(lines[4]).toContain("12.3");
    expect(lines[5]).toContain("-5.6");
  });

  it("omits camera-stats lines (not blank placeholders) when stats is null", () => {
    const lines = formatHudLines({ blurSigma: 0, noiseSigma: 0, exposureOffset: 0 }, null);
    expect(lines).toHaveLength(3);
  });
});
