// Trailing-edge throttle: at most one call per `intervalMs`, but the LAST
// call made during a throttled window is never dropped — it fires once
// `intervalMs` has elapsed since the previous invocation. Used by the video
// scrubber (Task 5e) for two independent rates: syncing the displayed
// slider position from `currentTime` during playback (~10Hz, so the UI
// doesn't re-render at video-frame-rate), and throttling `seek()` calls
// while the user drags the range input (~10/s, so a fast drag doesn't fire
// a `currentTime` write — and therefore a decode-triggering `seeked`/rVFC
// event — on every `pointermove`).
//
// Deliberately NOT a generic npm-style throttle: `flush`/`cancel` are the
// only extras callers need (the scrubber calls `cancel()` on drag release,
// then seeks the final position directly, bypassing the throttle entirely
// so the last position is never subject to the trailing delay).
export interface Throttled<Args extends unknown[]> {
  (...args: Args): void;
  /** Drop any pending trailing call without invoking it. */
  cancel(): void;
  /** Invoke a pending trailing call immediately, if one is scheduled. */
  flush(): void;
}

/**
 * Wrap `fn` so it's called at most once per `intervalMs`. A call inside the
 * window is remembered as "pending" and fires on a trailing timer once the
 * window elapses (never silently dropped) — matching the "last position
 * always wins" requirement for drag-seeking.
 *
 * `now` is injectable (defaults to `Date.now`) so tests can drive the clock
 * deterministically without real timers.
 */
export function throttle<Args extends unknown[]>(
  fn: (...args: Args) => void,
  intervalMs: number,
  now: () => number = Date.now,
): Throttled<Args> {
  let lastCallAt = -Infinity;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pendingArgs: Args | null = null;

  const clearTimer = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  };

  const invoke = (args: Args) => {
    lastCallAt = now();
    pendingArgs = null;
    fn(...args);
  };

  const throttled = (...args: Args) => {
    const elapsed = now() - lastCallAt;
    if (elapsed >= intervalMs) {
      clearTimer();
      invoke(args);
      return;
    }
    pendingArgs = args;
    if (timer === null) {
      timer = setTimeout(() => {
        timer = null;
        if (pendingArgs) invoke(pendingArgs);
      }, intervalMs - elapsed);
    }
  };

  throttled.cancel = () => {
    clearTimer();
    pendingArgs = null;
  };

  throttled.flush = () => {
    if (pendingArgs) {
      clearTimer();
      invoke(pendingArgs);
    }
  };

  return throttled;
}
