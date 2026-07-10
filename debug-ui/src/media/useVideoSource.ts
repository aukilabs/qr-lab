// Video-mode source loader: owns a hidden `<video>` element for a
// `File`/URL source, plays/steps it, and hands every *presented* frame's
// RGBA readout to `onFrame` — `App.tsx` feeds that straight into the same
// scan pipeline `useImageSource` uses, relying on `ScannerClient`'s
// latest-wins queue (Task 2) to drop frames the scanner can't keep up with
// rather than building an unbounded backlog.
//
// DOM-heavy (video element, canvas, `requestVideoFrameCallback`) — like
// `useImageSource`, left to manual QA per this vitest config's `node`
// environment (no jsdom); see Task 6's report for the checklist.
import { useEffect, useRef, useState } from "react";
import { clampTime } from "./videoTime";

/** Fallback step size for prev/next-frame seeking when the browser exposes
 * no better estimate of the video's actual frame rate — see the module
 * doc comment on why a fixed step (not measured fps) is enough here. */
const DEFAULT_FRAME_SECONDS = 1 / 30;

export interface VideoSourceState {
  /** Attach to a `<video muted playsInline ref={videoRef} />` in the
   * consumer's tree — the hook drives it imperatively (src, play/pause,
   * seeking) but doesn't render it itself, so the caller controls where
   * (or whether) it's visible. */
  videoRef: React.RefObject<HTMLVideoElement | null>;
  width: number;
  height: number;
  duration: number;
  currentTime: number;
  /** Count of frames handed to `onFrame` so far this source's lifetime —
   * resets to 0 on every `input` change. */
  frameIndex: number;
  playing: boolean;
  /** `false` when the browser lacks `HTMLVideoElement.
   * requestVideoFrameCallback` (Safari as of recent versions; older
   * browsers) — the hook falls back to `timeupdate`/`seeked` events, which
   * fire far less often than once per presented frame, so continuous
   * per-frame scanning during playback degrades to a few samples/sec
   * instead of matching the video's actual frame rate. Surfaced so the UI
   * can warn about the degraded mode instead of silently under-sampling. */
  supportsFrameCallback: boolean;
  error: string | null;
  play(): void;
  pause(): void;
  /** Step one frame forward (`1`) or backward (`-1`) while paused. Pauses
   * playback first if it was running. */
  stepFrame(direction: 1 | -1): void;
  /** Seek directly to `time` (seconds), clamped to `[0, duration]` — the
   * scrubber's drag/click/keyboard-release path. Unlike `stepFrame`, does
   * NOT pause playback itself (the scrubber component pauses explicitly on
   * drag start, matching the brief's "pause on scrub start, stay paused"
   * UX) — a caller that wants "seek without touching play state" (e.g. a
   * click on the bar while paused) gets exactly that. */
  seek(time: number): void;
  /** Re-deliver the CURRENT frame to `onFrame` without seeking or touching
   * play state (Plan 6): robust mode's "capture on paused frames only"
   * policy scans playing frames without visualization payload, so pausing
   * must re-scan the frame the user stopped on — with capture — to light
   * up the filmstrip/baseline overlays for it. No-ops before the first
   * frame has decoded (same guard as every other capture path). */
  recapture(): void;
}

export interface VideoFrame {
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
  /** The video's own clock at the moment this frame was presented
   * (`HTMLVideoElement.currentTime` in the fallback path,
   * `VideoFrameCallbackMetadata.mediaTime` when `rVFC` is available). */
  mediaTime: number;
}

/**
 * Load `input` into a managed `<video>` element and report every presented
 * frame to `onFrame`. `onFrame` is read through a ref internally so
 * passing a fresh closure each render doesn't tear down and re-arm the
 * frame-callback chain — only `input` identity does that.
 */
export function useVideoSource(
  input: File | Blob | string | null,
  onFrame: (frame: VideoFrame) => void,
): VideoSourceState {
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const onFrameRef = useRef(onFrame);
  onFrameRef.current = onFrame;

  const [width, setWidth] = useState(0);
  const [height, setHeight] = useState(0);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [frameIndex, setFrameIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const supportsFrameCallback =
    typeof HTMLVideoElement !== "undefined" &&
    "requestVideoFrameCallback" in HTMLVideoElement.prototype;

  /** Draw the video's current frame into the (lazily-created, reused)
   * offscreen canvas and hand its RGBA readout to `onFrame`. No-ops before
   * the video has decoded its first frame (`videoWidth`/`videoHeight` are
   * still 0). */
  const captureFrame = (mediaTime: number) => {
    const video = videoRef.current;
    if (!video || video.videoWidth === 0 || video.videoHeight === 0) return;

    let canvas = canvasRef.current;
    if (!canvas) {
      canvas = document.createElement("canvas");
      canvasRef.current = canvas;
    }
    if (canvas.width !== video.videoWidth) canvas.width = video.videoWidth;
    if (canvas.height !== video.videoHeight) canvas.height = video.videoHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.drawImage(video, 0, 0);
    const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height);

    onFrameRef.current({
      rgba: imageData.data,
      width: canvas.width,
      height: canvas.height,
      mediaTime,
    });
    setFrameIndex((n) => n + 1);
    setCurrentTime(video.currentTime);
  };

  // Source lifecycle: point the <video> at `input` (via an object URL for
  // File/Blob sources, revoked on the next change/unmount; used directly
  // for a URL string), and reset per-source state.
  useEffect(() => {
    const video = videoRef.current;
    setFrameIndex(0);
    setPlaying(false);
    setError(null);
    setWidth(0);
    setHeight(0);
    setDuration(0);
    setCurrentTime(0);

    if (!video) return;
    video.pause();

    if (!input) {
      video.removeAttribute("src");
      video.load();
      return;
    }

    // Branch on the same check for both `objectUrl` and `video.src` (rather
    // than `objectUrl ?? input`) — TS can't carry the narrowing from one
    // ternary into a later expression, so `objectUrl ?? input` widens back
    // to `string | File | Blob`, which isn't assignable to `.src: string`.
    const objectUrl = typeof input === "string" ? null : URL.createObjectURL(input);
    video.src = typeof input === "string" ? input : objectUrl!;
    video.load();

    return () => {
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [input]);

  // Native <video> event wiring: metadata (dims/duration), play/pause/ended
  // sync (so external play()/pause() calls AND user-driven native controls,
  // if ever shown, keep `playing` accurate), and decode errors.
  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    const onLoadedMetadata = () => {
      setWidth(video.videoWidth);
      setHeight(video.videoHeight);
      setDuration(video.duration);
    };
    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    const onEnded = () => setPlaying(false);
    const onError = () => {
      setError(video.error?.message ?? "video failed to load");
    };

    video.addEventListener("loadedmetadata", onLoadedMetadata);
    video.addEventListener("play", onPlay);
    video.addEventListener("pause", onPause);
    video.addEventListener("ended", onEnded);
    video.addEventListener("error", onError);
    return () => {
      video.removeEventListener("loadedmetadata", onLoadedMetadata);
      video.removeEventListener("play", onPlay);
      video.removeEventListener("pause", onPause);
      video.removeEventListener("ended", onEnded);
      video.removeEventListener("error", onError);
    };
  }, []);

  // Frame capture, primary path: a self-re-arming requestVideoFrameCallback
  // chain. Fires once per frame the compositor actually presents —
  // whether that's driven by playback (many calls/sec) or a single
  // `stepFrame` seek while paused (exactly one call, which doubles as the
  // "confirmation" that the seek landed and decoded, per the brief) —
  // since rVFC fires for both, no special-casing is needed between the two
  // call sites below.
  useEffect(() => {
    const video = videoRef.current;
    if (!video || !supportsFrameCallback) return;

    let cancelled = false;
    let handle: number | null = null;
    const onVideoFrame: VideoFrameRequestCallback = (_now, metadata) => {
      if (cancelled) return;
      captureFrame(metadata.mediaTime);
      handle = video.requestVideoFrameCallback(onVideoFrame);
    };
    handle = video.requestVideoFrameCallback(onVideoFrame);

    return () => {
      cancelled = true;
      if (handle != null) video.cancelVideoFrameCallback(handle);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- captureFrame reads only refs; re-arms per `input` (a new element load), not per render
  }, [input, supportsFrameCallback]);

  // Frame capture, fallback path (no rVFC support): `timeupdate` covers
  // playback (browsers fire it a few times/sec, not once per frame — a
  // known degradation, surfaced via `supportsFrameCallback`), `seeked`
  // covers `stepFrame`'s single-frame jumps.
  useEffect(() => {
    const video = videoRef.current;
    if (!video || supportsFrameCallback) return;

    const onTimeUpdate = () => captureFrame(video.currentTime);
    const onSeeked = () => captureFrame(video.currentTime);
    video.addEventListener("timeupdate", onTimeUpdate);
    video.addEventListener("seeked", onSeeked);
    return () => {
      video.removeEventListener("timeupdate", onTimeUpdate);
      video.removeEventListener("seeked", onSeeked);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- captureFrame reads only refs
  }, [input, supportsFrameCallback]);

  const play = () => {
    videoRef.current?.play().catch((err: unknown) => {
      setError(err instanceof Error ? err.message : String(err));
    });
  };

  const pause = () => {
    videoRef.current?.pause();
  };

  const stepFrame = (direction: 1 | -1) => {
    const video = videoRef.current;
    if (!video || !Number.isFinite(video.duration)) return;
    video.pause();
    video.currentTime = clampTime(video.currentTime + direction * DEFAULT_FRAME_SECONDS, video.duration);
  };

  const seek = (time: number) => {
    const video = videoRef.current;
    if (!video || !Number.isFinite(video.duration)) return;
    video.currentTime = clampTime(time, video.duration);
  };

  const recapture = () => {
    captureFrame(videoRef.current?.currentTime ?? 0);
  };

  return {
    videoRef,
    width,
    height,
    duration,
    currentTime,
    frameIndex,
    playing,
    supportsFrameCallback,
    error,
    play,
    pause,
    stepFrame,
    seek,
    recapture,
  };
}
