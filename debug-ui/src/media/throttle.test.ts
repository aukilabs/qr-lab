import { describe, expect, it, vi } from "vitest";
import { throttle } from "./throttle";

describe("throttle", () => {
  it("invokes immediately on the first call", () => {
    const fn = vi.fn();
    const t = throttle(fn, 100, () => 0);
    t(1);
    expect(fn).toHaveBeenCalledExactlyOnceWith(1);
  });

  it("drops calls inside the window but schedules a trailing call with the LAST args (never silently loses the final position)", () => {
    vi.useFakeTimers();
    let clock = 0;
    const fn = vi.fn();
    const t = throttle(fn, 100, () => clock);

    t("a"); // t=0, fires immediately
    clock = 30;
    t("b"); // inside window, deferred
    clock = 60;
    t("c"); // inside window, overwrites pending "b" with "c"
    expect(fn).toHaveBeenCalledTimes(1);

    // A trailing timer was scheduled once (by "b"; "c" only overwrote its
    // pending args, since one was already pending) for `intervalMs - elapsed`
    // real timer-ms out from when "b" was called — comfortably covered by
    // advancing 100ms of (fake) real time.
    vi.advanceTimersByTime(100);
    expect(fn).toHaveBeenCalledTimes(2);
    expect(fn).toHaveBeenLastCalledWith("c"); // "b" was superseded, never fired on its own
    vi.useRealTimers();
  });

  it("allows an immediate call again once the interval has fully elapsed", () => {
    let clock = 0;
    const fn = vi.fn();
    const t = throttle(fn, 100, () => clock);

    t("a");
    clock = 150;
    t("b");
    expect(fn).toHaveBeenCalledTimes(2);
    expect(fn).toHaveBeenLastCalledWith("b");
  });

  it("cancel() drops a pending trailing call without invoking it", () => {
    vi.useFakeTimers();
    let clock = 0;
    const fn = vi.fn();
    const t = throttle(fn, 100, () => clock);

    t("a");
    clock = 10;
    t("b"); // pending
    t.cancel();
    vi.advanceTimersByTime(200);
    expect(fn).toHaveBeenCalledTimes(1); // only "a" — "b" never fired
    vi.useRealTimers();
  });

  it("flush() invokes a pending trailing call immediately", () => {
    let clock = 0;
    const fn = vi.fn();
    const t = throttle(fn, 100, () => clock);

    t("a");
    clock = 10;
    t("b"); // pending
    t.flush();
    expect(fn).toHaveBeenCalledTimes(2);
    expect(fn).toHaveBeenLastCalledWith("b");
  });

  it("flush() is a no-op when nothing is pending", () => {
    const fn = vi.fn();
    const t = throttle(fn, 100, () => 0);
    t("a");
    t.flush();
    expect(fn).toHaveBeenCalledTimes(1);
  });
});
