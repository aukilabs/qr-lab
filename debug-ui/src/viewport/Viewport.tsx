import {
  useEffect,
  useRef,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  fitToView,
  identity,
  pan,
  screenToImage,
  zoomAt,
  type ViewTransform,
} from "./transform";

export interface ViewportProps {
  /** Source bitmap to render on the base canvas, or `null` while no image
   * is loaded (base canvas is just cleared in that case). */
  image: ImageBitmap | null;
  /** Called once per redraw with the overlay canvas's 2D context and the
   * current view transform; the ctx is in CSS-pixel space (already scaled
   * for devicePixelRatio and cleared) — layers project image-space data
   * through `imageToScreen(view, ...)` themselves before drawing.
   * Callers should memoize this callback; identity changes trigger a
   * redraw. */
  overlays: (ctx: CanvasRenderingContext2D, view: ViewTransform) => void;
  /** Fires on every pointer move with the image-space coordinates under
   * the cursor, and with `null` when the pointer leaves the viewport — for
   * a status-bar readout. */
  onCursorImagePos?: (p: [number, number] | null) => void;
}

const WHEEL_ZOOM_SENSITIVITY = 0.002;

/**
 * Two stacked canvases (base bitmap + overlay) inside a CSS-sized
 * container. View state (pan/zoom) lives in a ref and redraws are
 * rAF-batched rather than driven by React state, since mouse-move-driven
 * state updates would otherwise re-render the component on every pixel of
 * drag/wheel input.
 */
export function Viewport({ image, overlays, onCursorImagePos }: ViewportProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const baseCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const overlayCanvasRef = useRef<HTMLCanvasElement | null>(null);

  const viewRef = useRef<ViewTransform>(identity);
  const rafRef = useRef<number | null>(null);
  const dragRef = useRef<{ x: number; y: number } | null>(null);
  // Dimensions of the image the view was last auto-fit to, or `null` before
  // any bitmap has arrived. Compared (not just "is this a new source?")
  // because a resolution change swaps in a same-source bitmap at different
  // working dims — see the `[image, overlays]` effect below for why that
  // comparison is the deliberate trigger, not source identity.
  const lastFitDimsRef = useRef<{ width: number; height: number } | null>(null);

  // Latest-value refs so the imperative event handlers (attached once,
  // never re-subscribed) always see the current props without needing to
  // be recreated on every render.
  const imageRef = useRef(image);
  imageRef.current = image;
  const overlaysRef = useRef(overlays);
  overlaysRef.current = overlays;
  const onCursorRef = useRef(onCursorImagePos);
  onCursorRef.current = onCursorImagePos;

  const draw = () => {
    const base = baseCanvasRef.current;
    const overlay = overlayCanvasRef.current;
    if (!base || !overlay) return;
    const baseCtx = base.getContext("2d");
    const overlayCtx = overlay.getContext("2d");
    if (!baseCtx || !overlayCtx) return;

    const dpr = window.devicePixelRatio || 1;
    const view = viewRef.current;

    // Base canvas: draw the bitmap in image-pixel space, transformed by
    // dpr * view, so `drawImage(img, 0, 0)` places it correctly at any
    // zoom/pan. Nearest-neighbor (no smoothing) keeps QR modules crisp
    // when zoomed in instead of blurring into a smear.
    //
    // Clear policy (video flicker fix): only wipe the base canvas when we
    // are about to paint a valid bitmap, or when the image is intentionally
    // null (source cleared). If the previous ImageBitmap was already
    // closed (width 0 — common under robust-mode video where frames land
    // faster than React commits + rAF), leave the last painted pixels in
    // place. Clearing-then-skipping produced the intermittent black flash
    // on the feed during robust playback.
    baseCtx.setTransform(1, 0, 0, 1, 0, 0);
    const img = imageRef.current;
    // `img.width > 0` guards a close race: a re-scan of the same source
    // (resolution change, robust-panel edit) closes the previous
    // ImageBitmap before React commits the new one, and a draw already
    // queued via rAF would then throw InvalidStateError ("image source is
    // detached") on the closed bitmap — which reports width 0.
    if (img && img.width > 0) {
      baseCtx.clearRect(0, 0, base.width, base.height);
      baseCtx.setTransform(view.scale * dpr, 0, 0, view.scale * dpr, view.tx * dpr, view.ty * dpr);
      baseCtx.imageSmoothingEnabled = false;
      baseCtx.drawImage(img, 0, 0);
    } else if (!img) {
      baseCtx.clearRect(0, 0, base.width, base.height);
    }

    // Overlay canvas: only dpr-scaled, *not* by view — layers draw in
    // screen (CSS-pixel) space and are responsible for projecting their
    // image-space data through `imageToScreen(view, ...)` themselves (see
    // Task 4's registry/layers).
    overlayCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
    overlayCtx.clearRect(0, 0, overlay.width / dpr, overlay.height / dpr);
    overlaysRef.current(overlayCtx, view);
  };

  const scheduleRedraw = () => {
    if (rafRef.current != null) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      draw();
    });
  };

  // Size both canvases to the container (CSS pixels * devicePixelRatio),
  // keyed to container resize via ResizeObserver.
  useEffect(() => {
    const container = containerRef.current;
    const base = baseCanvasRef.current;
    const overlay = overlayCanvasRef.current;
    if (!container || !base || !overlay) return;

    const resize = () => {
      const rect = container.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;
      const w = Math.max(1, Math.round(rect.width * dpr));
      const h = Math.max(1, Math.round(rect.height * dpr));
      for (const canvas of [base, overlay]) {
        if (canvas.width !== w) canvas.width = w;
        if (canvas.height !== h) canvas.height = h;
        canvas.style.width = `${rect.width}px`;
        canvas.style.height = `${rect.height}px`;
      }
      scheduleRedraw();
    };

    resize();
    const ro = new ResizeObserver(resize);
    ro.observe(container);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- imperative resize handler, not prop-driven
  }, []);

  // Redraw when the image or overlay-drawing callback changes (e.g. new
  // scan data). Pan/zoom-driven redraws are scheduled directly from the
  // event handlers below, not through this effect.
  //
  // Auto-fit decision: before the redraw, fit the view whenever the
  // *display bitmap's dimensions* change from what the view was last fit
  // to — not merely "a new source was picked" and not "any new bitmap
  // committed" (that would re-fit, and so undo the user's pan/zoom, on
  // every re-scan/video frame at unchanged dims). Keying off dimension
  // change covers both cases the brief calls out with one rule: a brand
  // new source's first bitmap always differs from the previous (or null)
  // dims, so it always fits; and a resolution change swaps in a
  // same-source bitmap at different working dims, so it also re-fits
  // (deliberately — the old view's scale/pan was chosen for the old
  // working resolution and no longer matches). A same-dims re-scan of the
  // same source (video frames, ground-truth reload, etc.) leaves the
  // user's current pan/zoom alone. `lastFitDimsRef` is reset to `null`
  // when the bitmap is cleared (source change resets `image` to `null`
  // before the new source's first bitmap arrives), so the next bitmap —
  // even one that coincidentally matches the previous source's dims —
  // still triggers a fit.
  useEffect(() => {
    if (image) {
      const last = lastFitDimsRef.current;
      if (!last || last.width !== image.width || last.height !== image.height) {
        const container = containerRef.current;
        if (container) {
          const rect = container.getBoundingClientRect();
          viewRef.current = fitToView(image.width, image.height, rect.width, rect.height);
        }
        lastFitDimsRef.current = { width: image.width, height: image.height };
      }
    } else {
      lastFitDimsRef.current = null;
    }
    scheduleRedraw();
  }, [image, overlays]);

  useEffect(() => {
    return () => {
      if (rafRef.current != null) {
        cancelAnimationFrame(rafRef.current);
        // Must reset the ref, not just cancel: under StrictMode's
        // simulated unmount/remount this cleanup runs BEFORE the pending
        // rAF ever fires, and the canceled callback (the only other thing
        // that nulls the ref) never runs. Leaving the stale handle in
        // place made every scheduleRedraw() for the remounted component's
        // whole lifetime early-return on `rafRef.current != null` — a
        // permanently black viewport in dev (found by Task 6's
        // headless-browser smoke; invisible to the node-env unit tests).
        rafRef.current = null;
      }
    };
  }, []);

  const containerPoint = (clientX: number, clientY: number): [number, number] | null => {
    const container = containerRef.current;
    if (!container) return null;
    const rect = container.getBoundingClientRect();
    return [clientX - rect.left, clientY - rect.top];
  };

  const reportCursor = (screenPoint: [number, number] | null) => {
    const cb = onCursorRef.current;
    if (!cb) return;
    cb(screenPoint ? screenToImage(viewRef.current, screenPoint) : null);
  };

  // Wheel-to-zoom. Attached manually (not via JSX `onWheel`) because React
  // registers its wheel listeners as PASSIVE since v17, which makes
  // `preventDefault()` a silent no-op — the page would scroll / pinch-zoom
  // alongside our zoomAt. A native `{ passive: false }` listener is the
  // only way to actually consume the event. The handler reads everything
  // through refs (viewRef etc.), so binding once is safe; the container
  // div's identity is stable for the component's lifetime (a single,
  // unconditionally rendered element), so this effect never needs to
  // re-bind — if that ever changes, switch containerRef to a callback ref
  // and re-run this effect off the node.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const handleWheel = (e: globalThis.WheelEvent) => {
      e.preventDefault();
      const point = containerPoint(e.clientX, e.clientY);
      if (!point) return;
      const factor = Math.exp(-e.deltaY * WHEEL_ZOOM_SENSITIVITY);
      viewRef.current = zoomAt(viewRef.current, point, factor);
      scheduleRedraw();
      reportCursor(point);
    };

    container.addEventListener("wheel", handleWheel, { passive: false });
    return () => container.removeEventListener("wheel", handleWheel);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- handler reads only refs; container node is stable
  }, []);

  const handlePointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    dragRef.current = { x: e.clientX, y: e.clientY };
    e.currentTarget.style.cursor = "grabbing";
  };

  const handlePointerMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const point = containerPoint(e.clientX, e.clientY);
    const drag = dragRef.current;
    if (drag) {
      const dx = e.clientX - drag.x;
      const dy = e.clientY - drag.y;
      dragRef.current = { x: e.clientX, y: e.clientY };
      viewRef.current = pan(viewRef.current, dx, dy);
      scheduleRedraw();
    }
    reportCursor(point);
  };

  const endDrag = (e: ReactPointerEvent<HTMLDivElement>) => {
    dragRef.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    e.currentTarget.style.cursor = "grab";
  };

  const handlePointerLeave = () => {
    reportCursor(null);
  };

  const handleDoubleClick = () => {
    const container = containerRef.current;
    const img = imageRef.current;
    if (!container || !img) return;
    const rect = container.getBoundingClientRect();
    viewRef.current = fitToView(img.width, img.height, rect.width, rect.height);
    scheduleRedraw();
  };

  return (
    <div
      ref={containerRef}
      style={{
        position: "relative",
        width: "100%",
        height: "100%",
        overflow: "hidden",
        touchAction: "none",
        cursor: "grab",
      }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onPointerLeave={handlePointerLeave}
      onDoubleClick={handleDoubleClick}
    >
      <canvas
        ref={baseCanvasRef}
        style={{ position: "absolute", top: 0, left: 0, display: "block" }}
      />
      <canvas
        ref={overlayCanvasRef}
        style={{ position: "absolute", top: 0, left: 0, display: "block" }}
      />
    </div>
  );
}
