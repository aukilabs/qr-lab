// Source picker: drag-drop/file-pick for an arbitrary image or video, a
// golden-fixture (+ real-photo) dropdown fetched from the generated
// manifest, the working-resolution selector, and the re-scan button.
// Emits a `SourceDescriptor` describing *what* to load — `App.tsx` owns
// actually loading it (via `useImageSource`/`useVideoSource`) and running
// the scan pipeline, so this component stays free of scanning/decoding
// concerns and is just the picker UI.
import { useEffect, useState } from "react";

export interface FixtureEntry {
  /** Manifest/dropdown key — `real/`-prefixed for real-photo entries (see
   * `scripts/gen-fixture-manifest.mjs`). */
  name: string;
  /** Path relative to `fixtures/`, e.g. `"near_00.png"` or
   * `"real/real_1.png"` — fetch via `/fixtures/${png}` (see
   * `public/fixtures`, a symlink to `../../fixtures`). */
  png: string;
  /** Same convention as `png`, or `null` when this fixture has no ground
   * truth (real photos before regeneration). */
  json: string | null;
}

export type MediaKind = "image" | "video";

export type SourceDescriptor =
  | { kind: "file"; mediaKind: MediaKind; file: File }
  | { kind: "fixture"; mediaKind: "image"; fixture: FixtureEntry };

export const RESOLUTION_OPTIONS = ["full", 1920, 1280, 960, 640] as const;
export type ResolutionOption = (typeof RESOLUTION_OPTIONS)[number];
export const DEFAULT_RESOLUTION: ResolutionOption = 1280;

/** `ResolutionOption` -> `downscaleRgba`'s `maxDim` convention (`0` means
 * "no cap" — see `scanner/downscale.ts`). */
export function maxDimFor(resolution: ResolutionOption): number {
  return resolution === "full" ? 0 : resolution;
}

function parseResolutionOption(value: string): ResolutionOption {
  if (value === "full") return "full";
  const n = Number(value);
  const match = RESOLUTION_OPTIONS.find((r) => r === n);
  return match ?? DEFAULT_RESOLUTION;
}

const IMAGE_EXTENSIONS = [".png", ".jpg", ".jpeg"];
const VIDEO_EXTENSIONS = [".mp4", ".webm", ".mov"];

/** Classify a picked/dropped `File` by extension (falling back to its MIME
 * `type` prefix — a drag-dropped file's name isn't always reliable, e.g.
 * some OS file choosers). Returns `null` for anything unrecognized so
 * callers can reject it with a clear message instead of guessing. */
export function mediaKindForFile(file: File): MediaKind | null {
  const lowerName = file.name.toLowerCase();
  if (IMAGE_EXTENSIONS.some((ext) => lowerName.endsWith(ext))) return "image";
  if (VIDEO_EXTENSIONS.some((ext) => lowerName.endsWith(ext))) return "video";
  if (file.type.startsWith("image/")) return "image";
  if (file.type.startsWith("video/")) return "video";
  return null;
}

export interface SourcePanelProps {
  source: SourceDescriptor | null;
  onSourceChange: (source: SourceDescriptor) => void;
  resolution: ResolutionOption;
  onResolutionChange: (resolution: ResolutionOption) => void;
  onRescan: () => void;
  /** Disable the re-scan button — e.g. no source loaded yet, or video mode
   * (which already re-scans every presented frame; see `App.tsx`). */
  rescanDisabled: boolean;
}

/**
 * Fetches `/fixtures-manifest.json` once on mount. A fetch failure (e.g.
 * the generator never ran) surfaces as an empty list plus an error string
 * rather than throwing — the rest of the source panel (file pick/drop,
 * resolution, re-scan) stays usable without fixtures.
 */
function useFixtureManifest(): { fixtures: FixtureEntry[]; error: string | null } {
  const [fixtures, setFixtures] = useState<FixtureEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/fixtures-manifest.json")
      .then((res) => {
        if (!res.ok) throw new Error(`fixtures-manifest.json: HTTP ${res.status}`);
        return res.json() as Promise<unknown>;
      })
      .then((data) => {
        if (cancelled) return;
        if (!Array.isArray(data)) throw new Error("fixtures-manifest.json: expected an array");
        setFixtures(data as FixtureEntry[]);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { fixtures, error };
}

export function SourcePanel({
  source,
  onSourceChange,
  resolution,
  onResolutionChange,
  onRescan,
  rescanDisabled,
}: SourcePanelProps) {
  const { fixtures, error: manifestError } = useFixtureManifest();
  const [pickError, setPickError] = useState<string | null>(null);
  const [dragActive, setDragActive] = useState(false);

  const acceptFile = (file: File) => {
    const mediaKind = mediaKindForFile(file);
    if (!mediaKind) {
      setPickError(`unrecognized file type: "${file.name}" (expected png/jpg/jpeg or mp4/webm/mov)`);
      return;
    }
    setPickError(null);
    onSourceChange({ kind: "file", mediaKind, file });
  };

  const handleFileInput = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = ""; // allow re-picking the same file consecutively
    if (file) acceptFile(file);
  };

  const handleDrop = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    setDragActive(false);
    const file = e.dataTransfer.files[0];
    if (file) acceptFile(file);
  };

  const handleFixtureChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const name = e.target.value;
    const fixture = fixtures.find((f) => f.name === name);
    if (fixture) onSourceChange({ kind: "fixture", mediaKind: "image", fixture });
  };

  const currentFixtureName = source?.kind === "fixture" ? source.fixture.name : "";
  const currentFileLabel = source?.kind === "file" ? source.file.name : null;

  return (
    <section style={{ display: "flex", flexDirection: "column", gap: 10 }}>
      <h2 style={{ fontSize: 13, margin: 0, color: "#9ca3af", textTransform: "uppercase", letterSpacing: 0.5 }}>
        Source
      </h2>

      <div
        onDragOver={(e) => {
          e.preventDefault();
          setDragActive(true);
        }}
        onDragLeave={() => setDragActive(false)}
        onDrop={handleDrop}
        style={{
          border: `1px dashed ${dragActive ? "#4ade80" : "#4b5563"}`,
          borderRadius: 4,
          padding: "10px 8px",
          fontSize: 12,
          textAlign: "center",
          color: "#9ca3af",
          background: dragActive ? "rgba(74, 222, 128, 0.08)" : "transparent",
        }}
      >
        <div>Drag & drop an image or video here</div>
        <div style={{ margin: "6px 0" }}>or</div>
        <label style={{ cursor: "pointer", color: "#4ade80", textDecoration: "underline" }}>
          choose a file
          <input
            type="file"
            accept="image/png,image/jpeg,video/mp4,video/webm,video/quicktime"
            onChange={handleFileInput}
            style={{ display: "none" }}
          />
        </label>
        {currentFileLabel && (
          <div style={{ marginTop: 6, color: "#e5e7eb", overflowWrap: "anywhere" }}>{currentFileLabel}</div>
        )}
        {pickError && <div style={{ marginTop: 6, color: "#f87171" }}>{pickError}</div>}
      </div>

      <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
        <span style={{ color: "#9ca3af" }}>Golden fixture</span>
        <select value={currentFixtureName} onChange={handleFixtureChange}>
          <option value="" disabled>
            {fixtures.length === 0 ? "(loading…)" : "Choose a fixture…"}
          </option>
          {fixtures.map((f) => (
            <option key={f.name} value={f.name}>
              {f.name}
              {f.json ? "" : " (no ground truth)"}
            </option>
          ))}
        </select>
        {manifestError && <span style={{ color: "#f87171" }}>manifest: {manifestError}</span>}
      </label>

      <label style={{ display: "flex", flexDirection: "column", gap: 4, fontSize: 12 }}>
        <span style={{ color: "#9ca3af" }}>Working resolution</span>
        <select
          value={String(resolution)}
          onChange={(e) => onResolutionChange(parseResolutionOption(e.target.value))}
        >
          {RESOLUTION_OPTIONS.map((r) => (
            <option key={r} value={String(r)}>
              {r === "full" ? "Full" : `${r}px`}
            </option>
          ))}
        </select>
      </label>

      <button type="button" onClick={onRescan} disabled={rescanDisabled}>
        Re-scan
      </button>
    </section>
  );
}
