// Variant-buffer filmstrip (Plan 6, robust mode + capture only): one
// grayscale thumbnail per `snapshots` entry — the exact (≤320px) buffer
// the scanner saw at that ladder rung — labeled with the variant's name
// and framed in its stage color. Clicking a thumbnail selects it and shows
// it enlarged above the strip, the "what did the scanner actually see at
// this rung" view. Snapshots absent (capture off) renders a hint instead.
import { useEffect, useRef, useState } from "react";
import type { RobustSnapshot } from "../scanner/robust-types";
import { stageColor, variantKindLabel } from "../scanner/robust-types";

export interface FilmstripPanelProps {
  /** One entry per ladder variant, execution order — or `null` when the
   * last robust scan ran without capture (or none has run yet). */
  snapshots: RobustSnapshot[] | null;
  /** `true` when `snapshots` came from an EARLIER captured frame than the
   * one currently displayed — the "paused frames only" capture policy
   * scans playing video frames without capture, and the strip keeps the
   * last captured frame (badged) rather than flashing empty. */
  stale?: boolean;
  /** `true` in Plan 6 session (temporal) mode — the session video path
   * never captures ladder buffers, so the strip shows an explicit
   * unavailable hint instead of an "enable capture" one. */
  sessionMode?: boolean;
}

/** Paint a snapshot's tightly-packed grayscale bytes onto `canvas` at 1:1
 * buffer resolution (gray → opaque RGBA); CSS scales it for display. */
function drawSnapshot(canvas: HTMLCanvasElement, snapshot: RobustSnapshot): void {
  canvas.width = snapshot.width;
  canvas.height = snapshot.height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const image = ctx.createImageData(snapshot.width, snapshot.height);
  const rgba = image.data;
  const luma = snapshot.luma;
  for (let i = 0; i < luma.length; i++) {
    const v = luma[i]!;
    const o = i * 4;
    rgba[o] = v;
    rgba[o + 1] = v;
    rgba[o + 2] = v;
    rgba[o + 3] = 255;
  }
  ctx.putImageData(image, 0, 0);
}

function SnapshotCanvas({
  snapshot,
  style,
}: {
  snapshot: RobustSnapshot;
  style: React.CSSProperties;
}) {
  const ref = useRef<HTMLCanvasElement | null>(null);
  useEffect(() => {
    if (ref.current) drawSnapshot(ref.current, snapshot);
  }, [snapshot]);
  return <canvas ref={ref} style={style} />;
}

export function FilmstripPanel({ snapshots, stale = false, sessionMode = false }: FilmstripPanelProps) {
  const [selected, setSelected] = useState<number | null>(null);

  // A new scan's snapshots can be shorter than the previous selection (or
  // absent entirely) — clamp instead of pointing at a stale index.
  useEffect(() => {
    if (selected !== null && (!snapshots || selected >= snapshots.length)) {
      setSelected(null);
    }
  }, [snapshots, selected]);

  if (sessionMode) {
    return (
      <div className="filmstrip">
        <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
          filmstrip unavailable in session (temporal) mode — the session video path scans
          without capturing per-variant ladder buffers
        </span>
      </div>
    );
  }

  if (!snapshots || snapshots.length === 0) {
    return (
      <div className="filmstrip">
        <span style={{ fontSize: 12, color: "var(--text-muted)" }}>
          enable capture (Robust panel) to see per-variant ladder buffers
        </span>
      </div>
    );
  }

  const selectedSnapshot = selected !== null ? (snapshots[selected] ?? null) : null;

  return (
    <div className="filmstrip" style={stale ? { opacity: 0.55 } : undefined}>
      {stale && (
        <span style={{ fontSize: 11, color: "var(--warn)", fontFamily: "var(--font-mono)" }}>
          last captured frame — capture is paused during playback; pause the video to refresh
        </span>
      )}
      {selectedSnapshot && (
        <div style={{ display: "flex", alignItems: "flex-start", gap: 10 }}>
          <SnapshotCanvas
            snapshot={selectedSnapshot}
            style={{
              maxWidth: 480,
              width: "100%",
              height: "auto",
              imageRendering: "pixelated",
              border: `1px solid ${stageColor(selectedSnapshot.stage)}`,
              borderRadius: 1,
              boxShadow: `0 0 0 1px ${stageColor(selectedSnapshot.stage)}33`,
            }}
          />
          <div
            style={{
              fontSize: 12,
              color: "var(--text-secondary)",
              display: "flex",
              flexDirection: "column",
              gap: 4,
            }}
          >
            <span style={{ color: stageColor(selectedSnapshot.stage), fontFamily: "var(--font-mono)" }}>
              {variantKindLabel(selectedSnapshot.kind)}
            </span>
            <span style={{ color: "var(--text-muted)", fontFamily: "var(--font-mono)", fontSize: 11 }}>
              stage {selectedSnapshot.stage} · {selectedSnapshot.width}×{selectedSnapshot.height}
            </span>
            <button type="button" onClick={() => setSelected(null)}>
              close
            </button>
          </div>
        </div>
      )}

      <div style={{ display: "flex", gap: 8, overflowX: "auto", paddingBottom: 4 }}>
        {snapshots.map((snapshot, i) => (
          <button
            // Execution order is the identity (same rationale as
            // LadderPanel's row keys).
            key={i}
            type="button"
            onClick={() => setSelected(i)}
            style={{
              flex: "none",
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: 4,
              background: "transparent",
              border: "none",
              padding: 0,
              cursor: "pointer",
            }}
            title={`${variantKindLabel(snapshot.kind)} — stage ${snapshot.stage}`}
          >
            <SnapshotCanvas
              snapshot={snapshot}
              style={{
                height: 72,
                width: "auto",
                imageRendering: "pixelated",
                border: `1px solid ${i === selected ? stageColor(snapshot.stage) : "var(--border-default)"}`,
                borderRadius: 1,
                boxShadow: i === selected ? `0 0 0 1px ${stageColor(snapshot.stage)}66` : undefined,
              }}
            />
            <span
              style={{
                fontSize: 10,
                fontFamily: "var(--font-mono)",
                color: i === selected ? "var(--text-primary)" : "var(--text-muted)",
                maxWidth: 110,
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
              }}
            >
              {variantKindLabel(snapshot.kind)}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
