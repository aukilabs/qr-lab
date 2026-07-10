// Task 5e: the video-mode transport bar — play/pause + frame-step buttons
// (pre-existing, moved here verbatim from `App.tsx`) plus a new timeline
// scrubber (range input, click-to-jump + drag-to-scrub) and mm:ss.d time
// labels. Kept as its own component (rather than inline JSX in `App.tsx`)
// so its drag-driven local state (`scrubTime`/`dragging`) doesn't force the
// whole app to re-render on every `pointermove` — `App.tsx` only re-renders
// this component when `videoState`'s identity changes (every render, since
// `useVideoSource` returns a fresh object each time — see its own doc
// comment), same as before this task.
import { useEffect, useRef, useState } from "react";
import type { VideoSourceState } from "../media/useVideoSource";
import { throttle } from "../media/throttle";
import { clampTime, formatTime } from "../media/videoTime";

/** Cap on how often the scrubber's OWN state changes: the displayed
 * slider position/time label while playing (synced from `currentTime`,
 * which itself updates once per presented frame — see `useVideoSource`),
 * and `seek()` calls fired while dragging. Both throttled to the same
 * ~10Hz per the brief — frequent enough to feel live, far below
 * video-frame-rate re-render/seek churn. */
const THROTTLE_MS = 100;

export interface VideoControlsProps {
  videoState: VideoSourceState;
  /** Called once when the user grabs the scrubber (pointerdown — covers both
   * a click-to-jump and a drag). Plan 6 uses it to reset the temporal
   * session's cross-frame pool on a large seek; optional so non-session
   * callers can omit it. */
  onScrubStart?: () => void;
}

export function VideoControls({ videoState, onScrubStart }: VideoControlsProps) {
  // Note (review nit, accepted): starts at 0 rather than
  // `videoState.currentTime`, so a REMOUNT mid-video (e.g. leaving and
  // re-entering media mode) paints one frame with the thumb/label at
  // 00:00.0 before the sync effect below snaps it to the real position —
  // a single-frame cosmetic blip, not worth seeding from props (which
  // would silently couple initial state to render timing).
  const [scrubTime, setScrubTime] = useState(0);
  const [dragging, setDragging] = useState(false);

  // `useVideoSource` recreates `seek`/`stepFrame`/etc. every render (it
  // doesn't `useCallback` them), so the throttle instances below — created
  // once for this component's lifetime — read the latest closures through
  // a ref rather than capturing a stale one from whichever render they
  // were constructed in (same pattern `App.tsx` already uses for
  // `videoState.pause` in its mode-switch effect).
  const seekRef = useRef(videoState.seek);
  seekRef.current = videoState.seek;

  const [throttledSeek] = useState(() => throttle((time: number) => seekRef.current(time), THROTTLE_MS));
  const [throttledSync] = useState(() => throttle((time: number) => setScrubTime(time), THROTTLE_MS));

  // Disabled until `loadedmetadata` fires (duration starts at 0 and stays
  // there for a source with no metadata yet) — also covers the NaN/Infinity
  // edge case defensively, though `useVideoSource` shouldn't produce those
  // for `duration` in practice.
  const disabled = !Number.isFinite(videoState.duration) || videoState.duration <= 0;

  // Keep the displayed position following playback, throttled to ~10Hz —
  // NOT on every `currentTime` update (which land once per presented video
  // frame). Skipped while the user is actively dragging (their pointer
  // owns the displayed value until release) and reset instantly (bypassing
  // the throttle) whenever the source becomes unscrubbable, so a stale
  // scrub-bar position from a JUST-REPLACED source can't linger — the
  // source-lifecycle reset in `useVideoSource` zeroes `currentTime`/
  // `duration` together, so this fires in the same tick as that reset.
  useEffect(() => {
    if (disabled) {
      throttledSeek.cancel();
      throttledSync.cancel();
      setScrubTime(0);
      // A source swap mid-drag flips `disabled` true, which drops the
      // input's implicit pointer capture WITHOUT firing `pointerup` — if
      // `dragging` survived that, the `!dragging` sync branch below would
      // be gated forever (scrubber frozen until the user completed another
      // full press/release cycle on the new source). Reset it here;
      // `onPointerCancel` below covers the same class of capture loss for
      // paths that DO emit a pointer event.
      setDragging(false);
      return;
    }
    if (!dragging) throttledSync(videoState.currentTime);
  }, [videoState.currentTime, dragging, disabled, throttledSeek, throttledSync]);

  // Belt-and-braces cleanup on unmount (e.g. switching away from video
  // mode's file entirely) — the effect above already cancels on every
  // `disabled` transition, this only covers an unmount that skips that.
  useEffect(() => {
    return () => {
      throttledSeek.cancel();
      throttledSync.cancel();
    };
  }, [throttledSeek, throttledSync]);

  const handlePointerDown = () => {
    // Standard scrub UX: pause on grab, stay paused after release — the
    // user drives playback again explicitly (brief's decision, mirrors
    // the existing mode-switch pause behavior in `App.tsx`).
    if (videoState.playing) videoState.pause();
    setDragging(true);
    // A grab is the start of a large temporal jump — let the owner reset any
    // cross-frame session pool before the landed frames stream in (Plan 6).
    onScrubStart?.();
  };

  const handlePointerUp = () => {
    setDragging(false);
    // The final position must land exactly, not whenever the throttle
    // window next opens — cancel any pending throttled seek and issue the
    // authoritative one directly against the latest `seek` closure.
    //
    // Note (review nit, accepted): browsers can deliver one final
    // `input`/`change` for the release position AFTER `pointerup` — that
    // late `handleChange` re-enters `throttledSeek`, so the very last
    // position may land via the trailing throttle up to ~THROTTLE_MS
    // (~100ms) after release instead of through this direct call. Same
    // final value either way (the trailing call carries the latest args),
    // just marginally later — not worth suppressing.
    throttledSeek.cancel();
    seekRef.current(scrubTime);
  };

  const handlePointerCancel = () => {
    // Pointer capture lost without a `pointerup` (OS-level gesture
    // interruption, element disablement mid-drag on engines that do emit
    // `pointercancel` for it, etc.). Same reset as `handlePointerUp` but
    // WITHOUT the final authoritative seek: a cancelled drag never got a
    // deliberate release position, so leave the video wherever the last
    // throttled seek put it rather than treating an interruption as
    // intent. Any pending trailing seek is dropped for the same reason.
    setDragging(false);
    throttledSeek.cancel();
  };

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const value = e.target.valueAsNumber;
    if (Number.isNaN(value)) return;
    const clamped = clampTime(value, videoState.duration);
    // Unthrottled: the slider's own displayed position must track the
    // pointer 1:1 while dragging (and a plain click needs to jump
    // immediately) — only the actual `<video>` seek is rate-limited.
    setScrubTime(clamped);
    throttledSeek(clamped);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    // Range inputs step by a coarse default (browser-dependent, often a
    // whole unit) on Arrow keys — override with the existing frame-accurate
    // stepFrame so the scrubber's keyboard behavior matches the dedicated
    // frame-step buttons exactly, and `preventDefault` so the native step
    // doesn't ALSO fire (which would double-move the position).
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      videoState.stepFrame(-1);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      videoState.stepFrame(1);
    }
  };

  return (
    <div className="video-controls">
      <div className="video-controls-row">
        <button type="button" onClick={() => videoState.stepFrame(-1)} disabled={videoState.playing}>
          ◀ frame
        </button>
        <button
          type="button"
          onClick={() => (videoState.playing ? videoState.pause() : videoState.play())}
        >
          {videoState.playing ? "Pause" : "Play"}
        </button>
        <button type="button" onClick={() => videoState.stepFrame(1)} disabled={videoState.playing}>
          frame ▶
        </button>
        <span className="video-frame-counter">
          frame {videoState.frameIndex} · {formatTime(scrubTime)} / {formatTime(videoState.duration)}
        </span>
        {!videoState.supportsFrameCallback && (
          <span className="video-warn">no requestVideoFrameCallback — degraded capture rate</span>
        )}
      </div>
      <input
        type="range"
        className="video-scrub"
        aria-label="Video position"
        min={0}
        max={disabled ? 0 : videoState.duration}
        step="any"
        value={scrubTime}
        disabled={disabled}
        onPointerDown={handlePointerDown}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerCancel}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
      />
    </div>
  );
}
