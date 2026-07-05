// QR appearance + scene background + fixture-export controls (Plan 5d) —
// a second sidebar panel alongside `CameraSimControls.tsx`. Purely
// presentational (controlled inputs, no state of its own) — `Scene3D.tsx`
// owns the actual values, mirroring `CameraSimControls.tsx`'s own split
// (see that file's doc).
import { useRef, type ChangeEvent } from "react";
import { expectedInverted } from "./colorUtils";
import { QR_BG_ALPHA_RANGE } from "./consts";

export interface QrAppearanceValues {
  inkColor: string;
  bgColor: string;
  bgAlpha: number;
}

export interface QrAppearanceControlsProps {
  values: QrAppearanceValues;
  onChange: (values: QrAppearanceValues) => void;
  onBgImageChange: (file: File | null) => void;
  hasBgImage: boolean;
}

/**
 * Ink/background color pickers + background-alpha slider (feature 2/3),
 * a live "reads as: normal/inverted" + low-contrast warning readout
 * (`colorUtils.ts`'s `expectedInverted`), and the scene-background image
 * file picker (feature 1).
 */
export function QrAppearanceControls({
  values,
  onChange,
  onBgImageChange,
  hasBgImage,
}: QrAppearanceControlsProps) {
  const contrast = expectedInverted(values.inkColor, values.bgColor);
  // A native `<input type=file>` keeps showing its last-picked filename
  // even after the CONSUMING state clears (React never re-renders the
  // input's own internal display text just because a prop changed) — the
  // standard fix is resetting the DOM element's `.value` directly via a
  // ref when "clear" is clicked, so the picker visibly reflects "no file
  // chosen" again instead of a confusingly-stale filename next to an
  // actually-cleared background.
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const handleClear = () => {
    if (fileInputRef.current) fileInputRef.current.value = "";
    onBgImageChange(null);
  };
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <div style={{ display: "flex", gap: 8 }}>
        <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12, flex: 1 }}>
          <span style={{ color: "#9ca3af" }}>ink color</span>
          <input
            type="color"
            value={values.inkColor}
            onChange={(e) => onChange({ ...values, inkColor: e.target.value })}
          />
        </label>
        <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12, flex: 1 }}>
          <span style={{ color: "#9ca3af" }}>background color</span>
          <input
            type="color"
            value={values.bgColor}
            onChange={(e) => onChange({ ...values, bgColor: e.target.value })}
          />
        </label>
      </div>
      <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12 }}>
        <span style={{ display: "flex", justifyContent: "space-between", color: "#9ca3af" }}>
          <span>background alpha</span>
          <span style={{ fontVariantNumeric: "tabular-nums" }}>{values.bgAlpha.toFixed(2)}</span>
        </span>
        <input
          type="range"
          min={QR_BG_ALPHA_RANGE.min}
          max={QR_BG_ALPHA_RANGE.max}
          step={QR_BG_ALPHA_RANGE.step}
          value={values.bgAlpha}
          onChange={(e) => onChange({ ...values, bgAlpha: Number(e.target.value) })}
        />
      </label>
      <div
        data-testid="contrast-readout"
        style={{ fontSize: 11, color: contrast.lowContrast ? "#fbbf24" : "#6b7280" }}
      >
        reads as: {contrast.inverted ? "inverted" : "normal"}
        {contrast.lowContrast ? " · low contrast! (Δluma < 30)" : ""}
      </div>
      <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12 }}>
        <span style={{ color: "#9ca3af" }}>scene background image</span>
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          onChange={(e: ChangeEvent<HTMLInputElement>) => onBgImageChange(e.target.files?.[0] ?? null)}
        />
      </label>
      {hasBgImage && (
        <button type="button" onClick={handleClear} style={{ fontSize: 12 }}>
          clear background image
        </button>
      )}
    </div>
  );
}

export interface FixtureSaveControlsProps {
  name: string;
  onNameChange: (name: string) => void;
  onSave: () => void;
  saving: boolean;
  disabled: boolean;
  error: string | null;
}

/** Fixture-name text field + "Save as fixture" button (feature 6). */
export function FixtureSaveControls({
  name,
  onNameChange,
  onSave,
  saving,
  disabled,
  error,
}: FixtureSaveControlsProps) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
      <label style={{ display: "flex", flexDirection: "column", gap: 2, fontSize: 12 }}>
        <span style={{ color: "#9ca3af" }}>fixture name</span>
        <input
          type="text"
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          style={{ fontFamily: "monospace" }}
        />
      </label>
      <button type="button" onClick={onSave} disabled={disabled || saving} style={{ fontSize: 12 }}>
        {saving ? "saving…" : "save as fixture"}
      </button>
      {error && (
        <div className="banner banner-error" style={{ padding: 4, fontSize: 11 }}>
          {error}
        </div>
      )}
    </div>
  );
}
