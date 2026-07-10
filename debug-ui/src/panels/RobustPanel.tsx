// Robust-mode config panel (Plan 6): the master "Robust mode" toggle, the
// preset picker (values fetched from `robust_presets()` — the
// authoritative Rust constants, never hardcoded here), the seven ladder
// flag checkboxes, the variant budget, early exit, and the capture toggle.
// `App.tsx` owns all the state; this component is a dumb form over it,
// same as `SourcePanel`.
import type { RobustConfig, RobustPresets } from "../scanner/robust-types";

/**
 * When the filmstrip payload (per-variant buffer thumbnails, ~58 KB of
 * pixels per rung) is requested from the worker. Capture is pure
 * VISUALIZATION cost — detection results (and every overlay, which draws
 * from the always-present unified detections) are identical in every mode:
 * - `"off"` — never; leanest scans, filmstrip stays empty.
 * - `"paused"` (default) — still images and paused video frames only.
 *   Playing video frames scan without thumbnails (their serialization was
 *   the measured >200ms video round-trips), and pausing re-captures the
 *   frame you stopped on.
 * - `"always"` — every frame, including video playback (slow; for
 *   stepping through footage where per-frame filmstrips matter).
 */
export type RobustCaptureMode = "off" | "paused" | "always";

const CAPTURE_LABELS: Record<RobustCaptureMode, string> = {
  off: "off",
  paused: "paused frames only",
  always: "every frame (slow)",
};

export interface RobustPanelProps {
  enabled: boolean;
  onEnabledChange: (enabled: boolean) => void;
  /** `null` until the presets fetch resolves (the master toggle is
   * disabled until then — the config's initial value comes from the
   * presets, so there's nothing meaningful to enable before they load). */
  config: RobustConfig | null;
  onConfigChange: (config: RobustConfig) => void;
  captureMode: RobustCaptureMode;
  onCaptureModeChange: (mode: RobustCaptureMode) => void;
  presets: RobustPresets | null;
  /** Plan 6 session mode — the persistent-session video path. Rendered
   * always (App gates its actual USE to robust + video sources; the inputs
   * are inert for stills). */
  sessionEnabled: boolean;
  onSessionEnabledChange: (enabled: boolean) => void;
  sessionRotationPeriod: number;
  onSessionRotationPeriodChange: (period: number) => void;
  sessionPoolTtl: number;
  onSessionPoolTtlChange: (ttl: number) => void;
}

const FLAG_FIELDS: Array<{ key: keyof RobustConfig & string; label: string }> = [
  { key: "enableMultiScale", label: "multi-scale (pyramid)" },
  { key: "enableContrastNormalization", label: "contrast normalization" },
  { key: "enableShadowNormalization", label: "shadow normalization" },
  { key: "enableAdaptiveThresholding", label: "adaptive thresholding" },
  { key: "enableSharpening", label: "sharpening" },
  { key: "enableDeblur", label: "deblur" },
  { key: "enableLowResUpscaling", label: "low-res upscaling" },
];

const PRESET_LABELS = {
  baseline: "Baseline",
  robustFast: "Robust fast",
  robustFullBenchmark: "Robust full benchmark",
} as const;

type PresetKey = keyof typeof PRESET_LABELS;

function configsEqual(a: RobustConfig, b: RobustConfig): boolean {
  return (
    FLAG_FIELDS.every(({ key }) => a[key] === b[key]) &&
    a.maxVariantsPerFrame === b.maxVariantsPerFrame &&
    a.enableEarlyExit === b.enableEarlyExit
  );
}

/** Which preset the current config matches, or `"custom"` — derived, not
 * stored: any manual flag edit changes `config` away from every preset,
 * which flips the select to "custom" with no extra state to keep in sync. */
function presetFor(config: RobustConfig, presets: RobustPresets): PresetKey | "custom" {
  for (const key of Object.keys(PRESET_LABELS) as PresetKey[]) {
    if (configsEqual(config, presets[key])) return key;
  }
  return "custom";
}

const labelStyle: React.CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  fontSize: 12,
  color: "var(--text-secondary)",
};

const mutedStyle: React.CSSProperties = { color: "var(--text-muted)" };

export function RobustPanel({
  enabled,
  onEnabledChange,
  config,
  onConfigChange,
  captureMode,
  onCaptureModeChange,
  presets,
  sessionEnabled,
  onSessionEnabledChange,
  sessionRotationPeriod,
  onSessionRotationPeriodChange,
  sessionPoolTtl,
  onSessionPoolTtlChange,
}: RobustPanelProps) {
  const ready = presets !== null && config !== null;
  const preset = ready ? presetFor(config, presets) : "custom";

  const handlePresetChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    if (!presets) return;
    const value = e.target.value;
    if (value === "custom") return; // "custom" is a derived read-only state
    onConfigChange(presets[value as PresetKey]);
  };

  const handleRotationChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const n = Number(e.target.value);
    if (!Number.isFinite(n)) return;
    onSessionRotationPeriodChange(Math.max(1, Math.trunc(n)));
  };

  const handlePoolTtlChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const n = Number(e.target.value);
    if (!Number.isFinite(n)) return;
    onSessionPoolTtlChange(Math.max(1, Math.trunc(n)));
  };

  const setField = <K extends keyof RobustConfig>(key: K, value: RobustConfig[K]) => {
    if (!config) return;
    onConfigChange({ ...config, [key]: value });
  };

  const handleMaxVariantsChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const n = Number(e.target.value);
    if (!Number.isFinite(n)) return;
    setField("maxVariantsPerFrame", Math.max(0, Math.trunc(n)));
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <label style={labelStyle} title={ready ? undefined : "waiting for robust_presets()…"}>
        <input
          type="checkbox"
          checked={enabled}
          disabled={!ready}
          onChange={(e) => onEnabledChange(e.target.checked)}
        />
        Robust mode
      </label>

      <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
        <span style={mutedStyle}>Preset</span>
        <select value={preset} onChange={handlePresetChange} disabled={!ready}>
          {(Object.keys(PRESET_LABELS) as PresetKey[]).map((key) => (
            <option key={key} value={key}>
              {PRESET_LABELS[key]}
            </option>
          ))}
          <option value="custom" disabled>
            custom
          </option>
        </select>
      </label>

      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {FLAG_FIELDS.map(({ key, label }) => (
          <label key={key} style={labelStyle}>
            <input
              type="checkbox"
              checked={config?.[key] === true}
              disabled={!ready}
              onChange={(e) => setField(key, e.target.checked)}
            />
            {label}
          </label>
        ))}
      </div>

      <label style={labelStyle}>
        <span style={mutedStyle}>max variants/frame</span>
        <input
          type="number"
          min={0}
          step={1}
          value={config?.maxVariantsPerFrame ?? 0}
          disabled={!ready}
          onChange={handleMaxVariantsChange}
          style={{ width: 56 }}
        />
        <span style={mutedStyle}>(0 = unlimited)</span>
      </label>

      <label style={labelStyle}>
        <input
          type="checkbox"
          checked={config?.enableEarlyExit === true}
          disabled={!ready}
          onChange={(e) => setField("enableEarlyExit", e.target.checked)}
        />
        early exit
      </label>

      <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
        <span style={mutedStyle}>Capture ladder buffers</span>
        <select
          value={captureMode}
          onChange={(e) => onCaptureModeChange(e.target.value as RobustCaptureMode)}
        >
          {(Object.keys(CAPTURE_LABELS) as RobustCaptureMode[]).map((key) => (
            <option key={key} value={key}>
              {CAPTURE_LABELS[key]}
            </option>
          ))}
        </select>
      </label>
      <span style={{ ...mutedStyle, fontSize: 11 }}>
        capture feeds the filmstrip only (overlays always work — they draw from the unified
        detections every scan carries); "paused frames only" keeps video playback at pure
        ladder cost and re-captures the frame you pause on
      </span>

      <label style={labelStyle}>
        <input
          type="checkbox"
          checked={sessionEnabled}
          onChange={(e) => onSessionEnabledChange(e.target.checked)}
        />
        Session (temporal video)
      </label>
      <label style={labelStyle}>
        <span style={mutedStyle}>rotation period</span>
        <input
          type="number"
          min={1}
          step={1}
          value={sessionRotationPeriod}
          onChange={handleRotationChange}
          style={{ width: 56 }}
        />
      </label>
      <label style={labelStyle}>
        <span style={mutedStyle}>pool TTL frames</span>
        <input
          type="number"
          min={1}
          step={1}
          value={sessionPoolTtl}
          onChange={handlePoolTtlChange}
          style={{ width: 56 }}
        />
      </label>
      <span style={{ ...mutedStyle, fontSize: 11 }}>
        amortizes the ladder across video frames — lower per-frame cost, evidence pooled over
        time; still images ignore it
      </span>
    </div>
  );
}
