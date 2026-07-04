import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { buildManifest } from "./gen-fixture-manifest.mjs";

/** Tracks temp dirs created by {@link makeFixturesDir} so each test cleans
 * up after itself even on failure. */
let tempDirs = [];

afterEach(() => {
  for (const dir of tempDirs) {
    rmSync(dir, { recursive: true, force: true });
  }
  tempDirs = [];
});

/** Create a fresh temp directory (optionally with a `real/` subdir) and
 * touch each of `files` inside it — content doesn't matter, only presence,
 * since `buildManifest` only checks for `.png`/`.json` siblings by name. */
function makeFixturesDir(files, realFiles = []) {
  const dir = mkdtempSync(path.join(os.tmpdir(), "fixtures-manifest-test-"));
  tempDirs.push(dir);
  for (const f of files) writeFileSync(path.join(dir, f), "");
  if (realFiles.length > 0) {
    const realDir = path.join(dir, "real");
    mkdirSync(realDir);
    for (const f of realFiles) writeFileSync(path.join(realDir, f), "");
  }
  return dir;
}

describe("buildManifest", () => {
  it("pairs a golden fixture's png with its json sibling", () => {
    const dir = makeFixturesDir(["near_00.png", "near_00.luma", "near_00.json"]);
    expect(buildManifest(dir)).toEqual([
      { name: "near_00", png: "near_00.png", json: "near_00.json" },
    ]);
  });

  it("sets json to null when no sibling json exists (luma/json pairs not required)", () => {
    const dir = makeFixturesDir(["real_1.png", "real_1.luma"]);
    expect(buildManifest(dir)).toEqual([{ name: "real_1", png: "real_1.png", json: null }]);
  });

  it("ignores non-png files at the top level", () => {
    const dir = makeFixturesDir(["near_00.png", "near_00.json", "notes.txt", "README.md"]);
    expect(buildManifest(dir)).toEqual([
      { name: "near_00", png: "near_00.png", json: "near_00.json" },
    ]);
  });

  it("includes a real/ subdirectory's photos, name-prefixed and independently paired", () => {
    const dir = makeFixturesDir(
      ["near_00.png", "near_00.json"],
      ["real_1.png", "real_2.png", "real_2.json"],
    );
    expect(buildManifest(dir)).toEqual([
      { name: "near_00", png: "near_00.png", json: "near_00.json" },
      { name: "real/real_1", png: "real/real_1.png", json: null },
      { name: "real/real_2", png: "real/real_2.png", json: "real/real_2.json" },
    ]);
  });

  it("returns entries sorted by name", () => {
    const dir = makeFixturesDir(["far_00.png", "near_00.png", "combo_00.png"], ["real_1.png"]);
    expect(buildManifest(dir).map((e) => e.name)).toEqual([
      "combo_00",
      "far_00",
      "near_00",
      "real/real_1",
    ]);
  });

  it("does not recurse into subdirectories other than real/", () => {
    const dir = makeFixturesDir(["near_00.png"]);
    mkdirSync(path.join(dir, "other"));
    writeFileSync(path.join(dir, "other", "sneaky.png"), "");
    expect(buildManifest(dir)).toEqual([
      { name: "near_00", png: "near_00.png", json: null },
    ]);
  });

  it("returns an empty array for a fixtures dir with no real/ subdir and no pngs", () => {
    const dir = makeFixturesDir([]);
    expect(buildManifest(dir)).toEqual([]);
  });

  it("tolerates a missing real/ subdirectory entirely", () => {
    const dir = makeFixturesDir(["near_00.png", "near_00.json"]);
    expect(buildManifest(dir)).toEqual([
      { name: "near_00", png: "near_00.png", json: "near_00.json" },
    ]);
  });
});
