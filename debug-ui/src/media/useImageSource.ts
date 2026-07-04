// Image-mode source loader: a `File` (drag-drop/file-pick) or a URL string
// (a golden-fixture/real-photo PNG served from `public/fixtures/`) becomes
// a `getImageData` RGBA readout (what `App.tsx`'s scan pipeline feeds into
// `downscaleRgba`/`ScannerClient.scan`) plus the source's native
// dimensions. The `ImageBitmap` used for decoding is strictly an internal
// intermediate: it's `close()`d as soon as the pixels are read out, and
// deliberately NOT part of this hook's state — nothing consumed it (App
// builds its own working-res display bitmap from `rgba`), and keeping it
// alive leaked one full-resolution decode per source switch.
//
// DOM-heavy (canvas, `createImageBitmap`, `fetch`) — this vitest config runs
// tests under `environment: "node"` with no jsdom (see `vite.config.ts`),
// matching `Viewport.tsx`/`LayerPanel.tsx`'s precedent of leaving DOM-bound
// React code to manual QA rather than mocking a browser. See Task 6's
// report for the manual checklist this hook is on.
import { useEffect, useRef, useState } from "react";

export interface ImageSourceState {
  /** `getImageData(...).data` for the full decoded image, tightly packed
   * (stride === `width`), or `null` while nothing is loaded (or a load is
   * in flight / failed). */
  rgba: Uint8ClampedArray | null;
  width: number;
  height: number;
  loading: boolean;
  /** Set when decoding/fetching `input` failed; cleared on the next
   * successful load. `rgba` is `null` whenever this is set. */
  error: string | null;
}

const EMPTY_STATE: ImageSourceState = {
  rgba: null,
  width: 0,
  height: 0,
  loading: false,
  error: null,
};

/**
 * Decode `input` (a `File`/`Blob` from a picker, a URL string for a served
 * fixture PNG, or `null` for "no source") into an RGBA readout + native
 * dimensions. Re-runs whenever `input`'s identity changes; a load
 * superseded by a newer `input` before it finishes is dropped silently
 * (its state update never lands) so a slow fetch for a stale fixture can't
 * clobber a faster one for whatever the user picked next. The decode
 * bitmap is always closed before this settles — on success, cancellation,
 * and failure alike.
 */
export function useImageSource(input: File | Blob | string | null): ImageSourceState {
  const [state, setState] = useState<ImageSourceState>(EMPTY_STATE);
  // Guards the canvas element across re-runs so a rapid sequence of source
  // changes reuses one canvas instead of allocating a fresh one per load.
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    if (!input) {
      setState(EMPTY_STATE);
      return;
    }

    let cancelled = false;
    setState((s) => ({ ...s, loading: true, error: null }));

    void (async () => {
      let bitmap: ImageBitmap | null = null;
      try {
        const blob = typeof input === "string" ? await (await fetch(input)).blob() : input;
        bitmap = await createImageBitmap(blob);
        if (cancelled) return;

        let canvas = canvasRef.current;
        if (!canvas) {
          canvas = document.createElement("canvas");
          canvasRef.current = canvas;
        }
        canvas.width = bitmap.width;
        canvas.height = bitmap.height;
        const ctx = canvas.getContext("2d");
        if (!ctx) throw new Error("useImageSource: 2D canvas context unavailable");
        ctx.drawImage(bitmap, 0, 0);
        const imageData = ctx.getImageData(0, 0, bitmap.width, bitmap.height);

        if (cancelled) return;
        setState({
          rgba: imageData.data,
          width: bitmap.width,
          height: bitmap.height,
          loading: false,
          error: null,
        });
      } catch (err) {
        if (cancelled) return;
        setState({
          ...EMPTY_STATE,
          error: err instanceof Error ? err.message : String(err),
        });
      } finally {
        // The decode bitmap's only job (drawImage above) is done on every
        // path through here — success, cancellation, or failure — so
        // releasing its decoder memory in one place beats the previous
        // per-branch close() calls (which missed the success path
        // entirely, leaking one full-res decode per source switch).
        bitmap?.close();
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [input]);

  return state;
}
