#!/usr/bin/env python3
"""Render a sweep's measured results. Requires matplotlib (pip install matplotlib)."""
import argparse
import json
import math
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np


def stats(rows, mode="calibrated"):
    results = [r[mode] for r in rows]
    errors = sorted(r["returned_error"]["max"] for r in results if r["decoded"])
    return dict(n=len(rows), decoded=len(errors),
                within1=sum(r["within_1px"] for r in results),
                p95=errors[math.ceil(len(errors) * .95) - 1] if errors else None,
                worst=max(errors) if errors else None)


def percent(count, n):
    return 100 * count / n if n else float("nan")


def lens_key(row):
    return "pinhole" if row["lens"] == "pinhole" else f'{row["strength"]:g}'


def table(groups):
    lines = ["| Case group | Images | Raw decode | Calibrated decode | Decode + all corners ≤1 px | p95 worst corner | Worst corner |",
             "|---|---:|---:|---:|---:|---:|---:|"]
    for label, rows in groups:
        raw, cal = stats(rows, "uncorrected"), stats(rows)
        rate = lambda count: f'{count}/{cal["n"]} ({percent(count, cal["n"]):.1f}%)' if cal["n"] else "—"
        px = lambda value: "—" if value is None else f"{value:.3f} px"
        lines.append(f'| {label} | {cal["n"]} | {rate(raw["decoded"])} | {rate(cal["decoded"])} | {rate(cal["within1"])} | {px(cal["p95"])} | {px(cal["worst"])} |')
    return "\n".join(lines)


def save(fig, out, name):
    fig.savefig(out / f"{name}.png", dpi=150, facecolor="white")
    fig.savefig(out / f"{name}.svg", facecolor="white")
    plt.close(fig)


def verify_projection(out, rows, meta):
    # An independent implementation checks the lens mapping used for truth.
    # Requires a pinhole control at each pose; no OpenCV scanner is involved.
    import cv2

    key = lambda row: (row["tilt"], row["azimuth"], row["position"], row["margin"])
    controls = {key(r): r for r in rows if r["lens"] == "pinhole"}
    k = meta["camera"]
    camera = np.array([[k["fx"], 0, k["cx"]], [0, k["fy"], k["cy"]], [0, 0, 1.]])
    count, maximum = 0, 0.
    for row in rows:
        if row["lens"] != "fisheye":
            continue
        normalized = (np.array(controls[key(row)]["truth"]) - [k["cx"], k["cy"]]) / [k["fx"], k["fy"]]
        projected = cv2.fisheye.distortPoints(normalized.reshape(-1, 1, 2), camera,
                                             np.array([row["k1"], 0., 0., 0.])).reshape(-1, 2)
        maximum = max(maximum, float(np.abs(projected - row["truth"]).max()))
        count += len(projected)
    if not count or maximum > 1e-9:
        raise AssertionError(f"OpenCV projection check failed: {count} points, {maximum} px")
    result = dict(opencv_version=cv2.__version__, points=count, max_coordinate_difference_px=maximum)
    (out / "opencv-validation.json").write_text(json.dumps(result, indent=2) + "\n")
    print(result)


def analyze(out, verify_opencv=False):
    rows = [json.loads(s) for s in (out / "cases.jsonl").read_text().splitlines()]
    meta = json.loads((out / "metadata.json").read_text())
    if verify_opencv:
        verify_projection(out, rows, meta)
    clean = [r for r in rows if r["front_facing"] and not r["quiet_clipped"]]
    fish = [r for r in clean if r["lens"] == "fisheye"]
    lenses, tilts = meta["strengths"], meta["tilts"]
    labels = ["Pinhole" if s == "pinhole" else f"Fisheye {float(s):g}" for s in lenses]
    grids = [np.full((len(lenses), len(tilts)), np.nan) for _ in range(4)]
    for i, lens in enumerate(lenses):
        for j, tilt in enumerate(tilts):
            group = [r for r in clean if lens_key(r) == lens and r["tilt"] == tilt]
            raw, cal = stats(group, "uncorrected"), stats(group)
            for grid, value in zip(grids, [percent(raw["decoded"], raw["n"]),
                                          percent(cal["decoded"], cal["n"]),
                                          percent(cal["within1"], cal["n"]), cal["p95"]]):
                grid[i, j] = np.nan if value is None else value
    plt.rcParams.update({"font.family": "DejaVu Sans", "font.size": 10})
    fig, axes = plt.subplots(2, 2, figsize=(14, 9), layout="constrained")
    titles = ["Uncorrected · decoded (%)", "Calibrated · decoded (%)",
              "Calibrated · decoded AND every corner ≤1 px (%)",
              "Calibrated · p95 worst-corner error (px, decoded only)"]
    for idx, (ax, grid, title) in enumerate(zip(axes.flat, grids, titles)):
        limit = 1 if idx == 3 else 100
        cmap = plt.get_cmap("OrRd" if idx == 3 else "Blues").copy()
        cmap.set_bad("#e7e9ed")
        im = ax.imshow(grid, vmin=0, vmax=limit, cmap=cmap, aspect="auto")
        ax.set(xticks=range(len(tilts)), xticklabels=[f"{t}°" for t in tilts],
               yticks=range(len(lenses)), yticklabels=labels, title=title,
               xlabel="Plane tilt (actual viewing angle varies with position)")
        for (i, j), value in np.ndenumerate(grid):
            label = "—" if np.isnan(value) else (f"{value:.2f}" if idx == 3 else f"{value:.0f}")
            ax.text(j, i, label, ha="center", va="center", color="white" if value > limit * .6 else "#182230")
        fig.colorbar(im, ax=ax, shrink=.75, extend="max" if idx == 3 else "neither")
    fig.suptitle(f'QR lens stress test · {meta["resolution"]} × {meta["resolution"]} · version {meta["version"]}\n'
                 f'{len(clean):,} front-facing images with the entire quiet zone in the source frame; noise and blur off', fontsize=15)
    save(fig, out, "overview")

    angle_edges = [0, 30, 45, 55, 60, 65, 70, 75, 90]
    pitch_edges = [0, 1, 2, 3, 4, 6, 100]
    angle_groups = [(f"{a}–{b}°", [r for r in fish if a <= r["incidence_deg"] < b]) for a, b in zip(angle_edges, angle_edges[1:])]
    pitch_groups = [(f"{a}–{b}" if b < 100 else f"{a}+", [r for r in fish if a <= r["min_module_pitch_px"] < b]) for a, b in zip(pitch_edges, pitch_edges[1:])]
    fig, axes = plt.subplots(1, 2, figsize=(14, 5.5), layout="constrained")
    for ax, groups, title, xlabel in zip(axes, [angle_groups, pitch_groups],
            ["Decode falls with the viewing angle", "Small projected modules are harder to decode"],
            ["Actual incidence angle (normal vs. camera ray)", "Smallest sampled module pitch (sensor px)"]):
        x = np.arange(len(groups))
        for mode, label, color, marker in [("uncorrected", "Uncorrected decode", "#8994a3", "o"),
                                          ("calibrated", "Calibrated decode", "#176bba", "o"),
                                          ("within1", "Calibrated + corners ≤1 px", "#149169", "x")]:
            values = []
            for _, group in groups:
                s = stats(group, "calibrated" if mode == "within1" else mode)
                values.append(percent(s["within1" if mode == "within1" else "decoded"], s["n"]))
            ax.plot(x, values, marker=marker, label=label, color=color, linewidth=2)
        ax.set(xticks=x, xticklabels=[f"{label}\nn={len(group)}" for label, group in groups],
               ylim=(-3, 103), ylabel="Images (%)", xlabel=xlabel, title=title)
        ax.grid(axis="y", alpha=.2)
        ax.legend(fontsize=9)
    fig.suptitle("Fisheye cases only · full quiet zone · grouped observations, not independent trials", fontsize=14)
    save(fig, out, "limits")

    examples = json.loads((out / "examples.json").read_text())
    selected = []
    for reason, count in [("calibrated_rescue", 1), ("low_tilt_decode_failure", 1), ("worst_corner_error", 2)]:
        selected.extend([r for r in examples if r["reason"] == reason][:count])
    if selected:
        fig, axes = plt.subplots(1, len(selected), figsize=(4 * len(selected), 5), squeeze=False, layout="constrained")
        for ax, row in zip(axes.flat, selected):
            ax.imshow(plt.imread(out / f'{row["id"]}.png'))
            truth = np.array(row["truth"])
            ax.scatter(truth[:, 0], truth[:, 1], s=60, facecolors="none", edgecolors="#00d393", linewidths=2, label="Ground truth")
            error = "No decode"
            if row["calibrated"]["decoded"]:
                corners = np.array(row["calibrated"]["corners"])
                ax.scatter(corners[:, 0], corners[:, 1], marker="+", s=100, c="#ff38bf", linewidths=1.6, label="Returned corners")
                error = f'Worst corner {row["calibrated"]["returned_error"]["max"]:.2f} px'
            ax.set_xlim(max(-.5, truth[:, 0].min() - 45), min(meta["resolution"] - .5, truth[:, 0].max() + 45))
            ax.set_ylim(min(meta["resolution"] - .5, truth[:, 1].max() + 45), max(-.5, truth[:, 1].min() - 45))
            ax.set_title(f'{row["id"]} · {row["reason"].replace("_", " ")}\n'
                         f'Strength {row["strength"]} · tilt {row["tilt"]}° · {row["position"]}\n{error}', fontsize=10)
            ax.set_aspect("equal"); ax.tick_params(labelsize=8)
        fig.suptitle("Diagnostic crops · green circles = truth · pink crosses = calibrated corners", fontsize=14)
        save(fig, out, "examples")

    by_lens = [(label, [r for r in clean if lens_key(r) == key]) for key, label in zip(lenses, labels)]
    by_tilt = [(f"{tilt}°", [r for r in fish if r["tilt"] == tilt]) for tilt in tilts]
    by_position = [(p, [r for r in fish if r["position"] == p]) for p in meta["positions"]]
    front_fish = [r for r in rows if r["front_facing"] and r["lens"] == "fisheye"]
    clipping = [("Entire quiet zone visible", fish),
                ("Quiet zone clipped; modules intact", [r for r in front_fish if r["quiet_clipped"] and not r["module_clipped"]]),
                ("QR modules clipped", [r for r in front_fish if r["module_clipped"]])]
    report = f'''# Fisheye decoding and corner accuracy sweep

Generated from `{out.name}/cases.jsonl`; {len(rows):,} images and {len(rows) * 2:,} scans.

## Method

- Actual compiled WASM scanners, with and without known calibration. Full resolution, corner refinement enabled.
- Analytic planar QR renderer: {meta["samples"]}×{meta["samples"]} source supersampling, followed by the simulator's actual bilinear fisheye warp. This is not a capture of the WebGL UI.
- {meta["resolution"]}×{meta["resolution"]} pixels, 50° pinhole source field of view, nominal frontal QR width {meta["side"]:g} px, version {meta["version"]}, ECC M. One fixed payload, no blur or noise.
- Extra fisheye strength s means k1 = −0.5s, k2 = k3 = k4 = 0. Strength zero is equidistant fisheye; pinhole is a separate control. [OpenCV model](https://docs.opencv.org/4.x/db/d58/group__calib3d__fisheye.html).
- Tilt directions (azimuth): {meta["azimuths"]}. Positions: {", ".join(meta["positions"])}. Margins: {meta["margins"]} px, relative to the **quiet-zone boundary in the source pinhole frame**, not the warped sensor border. Negative margins test clipping.
- Main tables exclude back-facing codes and any clipped quiet zone. Corner error is Euclidean distance in original sensor pixels, in QR corner order. We score the worst of the four returned corners per decoded image. The p95 is nearest-rank and conditional on decoding. Decode + ≤1 px counts failures against the denominator.
- Actual incidence includes the camera ray's off-axis angle. Module pitch is the smallest of samples at nine points, not an exact global minimum.
- These deterministic, correlated synthetic cases are a stress grid, not a real-camera success probability. Timings in raw data are diagnostic, not a performance benchmark.
- The simulator warps an existing 50° pinhole render inward. It cannot generate scene content outside that source field of view. True wide-fisheye sensor-edge coverage remains untested.

## By lens (full quiet zone)

{table(by_lens)}

![Decoding and corner accuracy](overview.png)

## By plane tilt (fisheye only)

{table(by_tilt)}

## By actual viewing angle (fisheye only)

{table(angle_groups)}

## By smallest sampled module pitch (fisheye only)

{table(pitch_groups)}

![Viewing angle and module size](limits.png)

## By position (fisheye only)

{table(by_position)}

## Clipping (front-facing fisheye only)

{table(clipping)}

## Diagnostic fixtures

![Failure and accuracy examples](examples.png)

Selected PNGs and matching JSON contain exact parameters, calibration, ground-truth and returned corners. Selection rules preserve low-tilt misses, largest errors and calibration rescues; they are not a random sample.

## Reproduction

The [metadata](metadata.json) records all parameters, git HEAD, renderer and WASM hashes. The working tree can contain uncommitted changes, so HEAD alone does not identify the scanner build.

From `debug-ui`: `npm run bench:fisheye -- --out ../target/fisheye-sweep --samples 4`.

Then run `python scripts/analyze-fisheye-sweep.py ../target/fisheye-sweep` with matplotlib installed.

Raw results: [CSV](cases.csv), [JSONL](cases.jsonl), [summary](summary.json), [fixture index](examples.json).
'''
    (out / "report.md").write_text(report)
    (out / "analysis.json").write_text(json.dumps({"images": len(rows), "clean_fisheye": {mode: stats(fish, mode) for mode in ["uncorrected", "calibrated"]}}, indent=2) + "\n")
    print(out / "report.md")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--verify-opencv", action="store_true", help="Check ground truth with cv2; requires pinhole controls and opencv-python")
    args = parser.parse_args()
    analyze(args.directory.resolve(), args.verify_opencv)
