#!/usr/bin/env node
// Generates `public/fixtures-manifest.json`, the golden-fixture (and
// real-photo) index `SourcePanel.tsx` fetches to populate its fixture
// dropdown. Runs automatically before `npm run dev` / `npm run build` (npm's
// pre<script> lifecycle hook — see package.json) so the manifest is always
// fresh relative to `../fixtures` on disk.
//
// Serving: `debug-ui/public/fixtures` is a symlink to `../../fixtures`
// (checked into git as a symlink — see debug-ui/public/fixtures), so both
// `npm run dev` (Vite serves `public/` verbatim, following the symlink) and
// `vite build` + `vite preview` (Vite copies `public/` into `dist/` at
// build time; it follows the symlink and copies the *contents*, so the
// build output is self-contained without depending on the symlink at
// runtime) serve the actual fixture files at `/fixtures/<relPath>`. Entries
// in this manifest are paths *relative to `fixtures/`* (e.g. `"near_00.png"`,
// `"real/real_1.png"`) — consumers prepend `/fixtures/` to build the actual
// fetch URL, keeping the manifest's shape independent of exactly how it's
// served.
import { existsSync, mkdirSync, readdirSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const PNG_EXT = ".png";
const JSON_EXT = ".json";

/**
 * One fixture: `name` is the manifest/dropdown key (`real/`-prefixed for
 * real-photo entries so they're visually distinguishable from golden
 * fixtures without a separate grouping field), `png`/`json` are paths
 * relative to `fixturesDir` — `json` is `null` when no sibling `.json`
 * exists (real photos before ground truth has been regenerated; golden
 * fixtures always have one). `.luma` siblings aren't referenced — no
 * manifest consumer needs the raw luma plane.
 * @typedef {{ name: string, png: string, json: string | null }} FixtureEntry
 */

/**
 * Collect one directory's `.png` files (non-recursive — subdirectories
 * other than the caller-controlled `real/` walk in {@link buildManifest}
 * are ignored) into `FixtureEntry` objects, `name`/`png`/`json` prefixed by
 * `namePrefix` when given (e.g. `"real"` for the `real/` subdir).
 * @param {string} dir
 * @param {string} namePrefix
 * @returns {FixtureEntry[]}
 */
function collectDir(dir, namePrefix) {
  if (!existsSync(dir)) return [];

  const prefixed = (s) => (namePrefix ? `${namePrefix}/${s}` : s);
  const entries = [];

  for (const entryName of readdirSync(dir)) {
    const full = path.join(dir, entryName);
    if (statSync(full).isDirectory()) continue; // only an explicit real/ pass recurses
    if (!entryName.endsWith(PNG_EXT)) continue;

    const stem = entryName.slice(0, -PNG_EXT.length);
    const jsonName = `${stem}${JSON_EXT}`;
    const hasJson = existsSync(path.join(dir, jsonName));

    entries.push({
      name: prefixed(stem),
      png: prefixed(entryName),
      json: hasJson ? prefixed(jsonName) : null,
    });
  }

  return entries;
}

/**
 * Build the fixture manifest by walking `fixturesDir`: `.png` files
 * directly inside it, plus (separately) one level into a `real/`
 * subdirectory if present — matching the fixtures layout described in
 * `tools/fixtures/README.md` (golden `name.png`/`.luma`/`.json` triples at
 * the top level, PNG-only real-photo captures under `real/`). Returns
 * entries sorted by `name`.
 * @param {string} fixturesDir
 * @returns {FixtureEntry[]}
 */
export function buildManifest(fixturesDir) {
  const entries = [
    ...collectDir(fixturesDir, ""),
    ...collectDir(path.join(fixturesDir, "real"), "real"),
  ];
  entries.sort((a, b) => a.name.localeCompare(b.name));
  return entries;
}

function main() {
  const scriptDir = path.dirname(fileURLToPath(import.meta.url));
  const debugUiDir = path.dirname(scriptDir);
  const fixturesDir = path.resolve(debugUiDir, "..", "fixtures");
  const outDir = path.join(debugUiDir, "public");
  const outFile = path.join(outDir, "fixtures-manifest.json");

  const manifest = buildManifest(fixturesDir);

  mkdirSync(outDir, { recursive: true });
  writeFileSync(outFile, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`gen-fixture-manifest: wrote ${manifest.length} entries to ${outFile}`);
}

// Only run when invoked directly (`node gen-fixture-manifest.mjs` / the npm
// pre-hooks) — not when imported by the test file.
if (path.resolve(fileURLToPath(import.meta.url)) === path.resolve(process.argv[1] ?? "")) {
  main();
}
