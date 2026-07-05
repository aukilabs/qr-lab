// Scene-background image plane (Plan 5d): a large textured "floor" quad
// behind/coplanar-under the QR plane so a picked image reads as the
// environment the code floats in front of — and, crucially, is visible
// in the readback the scan loop actually scans (unlike a CSS background
// on the page, which never reaches the offscreen `WebGLRenderTarget`).
// No image picked -> renders nothing (the scene's existing dark
// `<color attach="background">` shows through, per the task brief: "no
// image -> current dark background").
//
// This component only CONSUMES an already-live object URL (or `null`) —
// it does not create/revoke one itself. `Scene3D.tsx` owns that lifecycle
// (`URL.createObjectURL(file)` on pick, `URL.revokeObjectURL` on
// change/unmount), matching the existing precedent of keeping DOM/URL
// lifecycle ownership at the top of a component tree rather than split
// across a leaf.
import { useEffect, useState } from "react";
import { DoubleSide, TextureLoader, type Texture } from "three";
import { BACKGROUND_PLANE_SCALE, BACKGROUND_PLANE_Z_OFFSET } from "./consts";

export interface BackgroundPlaneProps {
  /** An object URL (or any loadable image URL), or `null` for "no
   * background image picked". */
  imageUrl: string | null;
  /** The QR plane's own `physicalSize` (meters, full extent incl. quiet
   * zone) — the background plane is sized as a multiple of this (see
   * `BACKGROUND_PLANE_SCALE`). */
  physicalSize: number;
}

/**
 * A same-orientation, larger, slightly-behind plane textured with a
 * user-picked image — see `consts.ts`'s `BACKGROUND_PLANE_SCALE`/
 * `BACKGROUND_PLANE_Z_OFFSET` docs for the sizing/offset rationale.
 */
export function BackgroundPlane({ imageUrl, physicalSize }: BackgroundPlaneProps) {
  const [texture, setTexture] = useState<Texture | null>(null);

  useEffect(() => {
    if (!imageUrl) {
      setTexture(null);
      return;
    }
    let cancelled = false;
    let loaded: Texture | null = null;
    const loader = new TextureLoader();
    loader.load(
      imageUrl,
      (tex) => {
        if (cancelled) {
          tex.dispose();
          return;
        }
        loaded = tex;
        setTexture(tex);
      },
      undefined,
      (err) => {
        if (!cancelled) console.error("scene3d: background image load failed", err);
      },
    );
    return () => {
      cancelled = true;
      loaded?.dispose();
    };
  }, [imageUrl]);

  if (!texture) return null;

  const size = physicalSize * BACKGROUND_PLANE_SCALE;
  return (
    <mesh name="scene-background-plane" position={[0, 0, BACKGROUND_PLANE_Z_OFFSET]}>
      <planeGeometry args={[size, size]} />
      <meshBasicMaterial map={texture} toneMapped={false} side={DoubleSide} />
    </mesh>
  );
}
