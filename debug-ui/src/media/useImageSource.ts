// Image-mode source loader: a `File` (drag-drop/file-pick) or a URL string
// (a golden-fixture/real-photo PNG served from `public/fixtures/`) becomes
// an `ImageBitmap` (for cheap re-decodes / thumbnails) plus a `getImageData`
// RGBA readout (what `App.tsx`'s scan pipeline actually feeds into
// `downscaleRgba`/`ScannerClient.scan`) and the source's native dimensions.
//
// DOM-heavy (canvas, `createImageBitmap`, `fetch`) — this vitest config runs
// tests under `environment: "node"` with no jsdom (see `vite.config.ts`),
// matching `Viewport.tsx`/`LayerPanel.tsx`'s precedent of leaving DOM-bound
// React code to manual QA rather than mocking a browser. See Task 6's
// report for the manual checklist this hook is on.
import { useEffect, useRef, useState } from "react";

export interface ImageSourceState {
  /** Full-resolution decoded bitmap, or `null` while nothing is loaded (or
   * a load is in flight / failed). Callers needing a *display* bitmap at
   * working resolution build one themselves from `rgba` via
   * `downscaleRgba` + `createImageBitmap` — this is always the ORIGINAL,
   * undownscaled image. */
  bitmap: ImageBitmap | null;
  /** `getImageData(...).data` for the full bitmap, tightly packed
   * (stride === `width`), or `null` alongside `bitmap`. */
  rgba: Uint8ClampedArray | null;
  width: number;
  height: number;
  loading: boolean;
  /** Set when decoding/fetching `input` failed; cleared on the next
   * successful load. `bitmap`/`rgba` are `null` whenever this is set. */
  error: string | null;
}

const EMPTY_STATE: ImageSourceState = {
  bitmap: null,
  rgba: null,
  width: 0,
  height: 0,
  loading: false,
  error: null,
};

/**
 * Decode `input` (a `File`/`Blob` from a picker, a URL string for a served
 * fixture PNG, or `null` for "no source") into a bitmap + RGBA readout.
 * Re-runs whenever `input`'s identity changes; a load superseded by a newer
 * `input` before it finishes is dropped silently (its bitmap is `close()`d
 * to release the decoder resources, and its state update never lands) so a
 * slow fetch for a stale fixture can't clobber a faster one for whatever
 * the user picked next.
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
        if (cancelled) {
          bitmap.close();
          return;
        }

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

        if (cancelled) {
          bitmap.close();
          return;
        }
        setState({
          bitmap,
          rgba: imageData.data,
          width: bitmap.width,
          height: bitmap.height,
          loading: false,
          error: null,
        });
      } catch (err) {
        bitmap?.close();
        if (cancelled) return;
        setState({
          ...EMPTY_STATE,
          error: err instanceof Error ? err.message : String(err),
        });
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [input]);

  return state;
}
