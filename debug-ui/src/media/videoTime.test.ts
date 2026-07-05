import { describe, expect, it } from "vitest";
import { clampTime, formatTime } from "./videoTime";

describe("formatTime", () => {
  it("formats sub-minute durations as mm:ss.d", () => {
    expect(formatTime(0)).toBe("00:00.0");
    expect(formatTime(5.5)).toBe("00:05.5");
    expect(formatTime(59.9)).toBe("00:59.9");
  });

  it("carries seconds into minutes", () => {
    expect(formatTime(60)).toBe("01:00.0");
    expect(formatTime(63.4)).toBe("01:03.4");
    expect(formatTime(125.25)).toBe("02:05.3"); // .25 rounds to .3 (nearest tenth)
  });

  it("grows minutes unboundedly for hours-long sources instead of wrapping to h:mm:ss", () => {
    // 1h 2m 3.4s = 3723.4s
    expect(formatTime(3723.4)).toBe("62:03.4");
    // 10 hours = 36000s
    expect(formatTime(36000)).toBe("600:00.0");
  });

  it("rounds to the nearest tenth without float noise tipping into the next second", () => {
    expect(formatTime(59.9999999996)).toBe("01:00.0");
    expect(formatTime(0.04999999999)).toBe("00:00.0");
  });

  it("renders non-finite or negative input as a placeholder instead of NaN", () => {
    expect(formatTime(NaN)).toBe("--:--.-");
    expect(formatTime(Infinity)).toBe("--:--.-");
    expect(formatTime(-1)).toBe("--:--.-");
  });
});

describe("clampTime", () => {
  it("clamps into [0, duration]", () => {
    expect(clampTime(5, 10)).toBe(5);
    expect(clampTime(-5, 10)).toBe(0);
    expect(clampTime(15, 10)).toBe(10);
    expect(clampTime(0, 10)).toBe(0);
    expect(clampTime(10, 10)).toBe(10);
  });

  it("treats a non-finite time as 0 before clamping", () => {
    expect(clampTime(NaN, 10)).toBe(0);
    expect(clampTime(-Infinity, 10)).toBe(0);
    expect(clampTime(Infinity, 10)).toBe(10);
  });

  it("treats a non-finite duration (no metadata yet) as no upper bound", () => {
    expect(clampTime(5, NaN)).toBe(5);
    expect(clampTime(5, Infinity)).toBe(5);
    expect(clampTime(-5, NaN)).toBe(0);
  });
});
