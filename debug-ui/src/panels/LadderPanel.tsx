// Ladder visualization (Plan 6): one row per `robust.variants` entry in
// execution order — stage-colored dot, variant label, a horizontal time
// bar proportional to the variant's share of the slowest rung, F/T/C
// (finders/triplets/codes) counts, and a "+N" badge on rungs that
// contributed codes no earlier rung had found (the rows that earned their
// cost — subtly highlighted). Footer: whole-ladder wall time, variants
// run, and the early-exit / budget-hit badges.
import type { RobustDetections } from "../scanner/robust-types";
import { stageColor, variantKindLabel } from "../scanner/robust-types";

export interface LadderPanelProps {
  /** Latest robust scan's ladder result, or `null` before any robust scan
   * has completed. */
  robust: RobustDetections | null;
}

function formatTotalMs(ns: number): string {
  return `${(ns / 1e6).toFixed(1)}ms`;
}

const rowStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  fontSize: 11,
  fontFamily: "var(--font-mono)",
  padding: "2px 4px",
  borderRadius: 2,
};

export function LadderPanel({ robust }: LadderPanelProps) {
  if (!robust) {
    return <div style={{ fontSize: 12, color: "var(--text-muted)" }}>no robust scan yet</div>;
  }

  const maxTotalNs = Math.max(...robust.variants.map((v) => v.total_ns), 1);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {robust.variants.map((v, i) => {
        const color = stageColor(v.stage);
        const earned = v.new_codes > 0;
        return (
          <div
            // Execution order is the identity here — the same kind can run
            // more than once in principle, so the index is the stable key.
            key={i}
            style={{
              ...rowStyle,
              background: earned ? "var(--accent-dim)" : "transparent",
            }}
          >
            <span
              style={{
                width: 7,
                height: 7,
                borderRadius: 1,
                background: color,
                flex: "none",
                boxShadow: earned ? `0 0 6px ${color}` : undefined,
              }}
            />
            <span
              style={{
                width: 110,
                flex: "none",
                overflow: "hidden",
                textOverflow: "ellipsis",
                whiteSpace: "nowrap",
                color: "var(--text-secondary)",
              }}
              title={variantKindLabel(v.kind)}
            >
              {variantKindLabel(v.kind)}
            </span>
            <span style={{ flex: 1, minWidth: 24 }}>
              <span
                style={{
                  display: "block",
                  height: 4,
                  borderRadius: 1,
                  width: `${Math.max((v.total_ns / maxTotalNs) * 100, 2)}%`,
                  background: color,
                  opacity: 0.85,
                }}
              />
            </span>
            <span style={{ color: "var(--text-muted)", flex: "none" }} title="finders / triplets / codes">
              {v.finders}F {v.triplets}T {v.codes}C
            </span>
            {earned && (
              <span
                style={{
                  flex: "none",
                  color: "var(--accent-text)",
                  background: "var(--accent)",
                  borderRadius: 1,
                  padding: "0 4px",
                  fontWeight: 600,
                  fontSize: 10,
                }}
                title={`${v.new_codes} code(s) no earlier rung found`}
              >
                +{v.new_codes}
              </span>
            )}
          </div>
        );
      })}

      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          marginTop: 6,
          paddingTop: 6,
          borderTop: "1px solid var(--border-subtle)",
          fontSize: 11,
          fontFamily: "var(--font-mono)",
          color: "var(--text-secondary)",
        }}
      >
        <span>total {formatTotalMs(robust.total_ns)}</span>
        <span style={{ color: "var(--text-muted)" }}>{robust.variants.length} variants</span>
        {robust.early_exited && <span style={{ color: "var(--accent)" }}>early exit ✓</span>}
        {robust.budget_exhausted && <span style={{ color: "var(--warn)" }}>budget hit</span>}
      </div>
    </div>
  );
}
