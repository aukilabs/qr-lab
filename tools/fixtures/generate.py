"""Generate the golden fixture suite. See README.md."""
import argparse
import dataclasses
import json
from pathlib import Path

import cv2
import numpy as np

import camera
import degrade
import render
import scenarios


def render_fixture(spec, intr):
    rng = np.random.default_rng(spec.seed)
    img = np.full((intr.height, intr.width), 128, np.uint8)
    truths = []
    for code in spec.codes:
        r, t = camera.make_pose(code.distance_m, code.tilt_deg,
                                code.tilt_azimuth_deg, code.inplane_deg,
                                code.image_point, intr)
        modules = render.make_symbol(code.payload, code.version, code.ecc,
                                     code.mirrored)
        render.render_code(img, intr, r, t, modules, code.physical_size_m,
                           render.Levels(), code.inverted, code.opaque_plate)
        corners = render.corners_px(intr, r, t, code.physical_size_m)
        n = modules.shape[0]
        module_px = float(np.linalg.norm(corners[1] - corners[0]) / n)
        truths.append({
            "payload": code.payload, "version": code.version,
            "ecc": code.ecc, "mirrored": code.mirrored,
            "physical_size_m": code.physical_size_m,
            "distance_m": code.distance_m, "tilt_deg": code.tilt_deg,
            "tilt_azimuth_deg": code.tilt_azimuth_deg,
            "inplane_deg": code.inplane_deg,
            "module_size_px": module_px,
            "corners_px": [[float(x), float(y)] for x, y in corners],
            "inverted": code.inverted, "opaque_plate": code.opaque_plate,
        })

    # Degradations apply in the order documented in degrade.py: optics
    # (motion/defocus) -> illumination -> resolution -> blur_sigma ->
    # occlusion -> noise -> jpeg. `deg is None` (all pre-plan6 specs) takes
    # exactly the legacy path, byte-identically.
    deg = spec.degradations
    if deg is not None:
        if deg.motion_blur is not None:
            mb = deg.motion_blur
            img = degrade.motion_blur(img, mb.length_px, mb.angle_deg,
                                      mb.curve)
        if deg.defocus_radius_px is not None:
            img = degrade.defocus_blur(img, deg.defocus_radius_px)
        if deg.shadow is not None:
            sh = deg.shadow
            img = degrade.shadow(img, sh.strength, sh.softness_px,
                                 sh.angle_deg, sh.offset_xy, sh.shape,
                                 sh.size_px)
        if deg.illum_gradient is not None:
            ig = deg.illum_gradient
            img = degrade.illum_gradient(img, ig.min_gain, ig.max_gain,
                                         ig.angle_deg)
        if deg.glare is not None:
            gl = deg.glare
            img = degrade.glare(img, gl.center_xy, gl.radius_px, gl.gain)
        if deg.contrast_scale is not None:
            img = degrade.contrast_compress(img, deg.contrast_scale)
        if deg.resolution is not None:
            img = degrade.resolution(img, deg.resolution.factor,
                                     deg.resolution.method)

    if spec.blur_sigma > 0:
        img = cv2.GaussianBlur(img, (0, 0), spec.blur_sigma)
    if deg is not None and deg.occlusion is not None:
        img = degrade.occlusion(img, deg.occlusion.rect_xyxy,
                                deg.occlusion.gray)
    if spec.noise_sigma > 0:
        if deg is not None and deg.shot_noise:
            # Signal-dependent (shot) noise: per-pixel sigma scales with
            # sqrt(signal), normalized to noise_sigma at mid-gray (128).
            sigma = spec.noise_sigma * np.sqrt(
                np.maximum(img.astype(np.float32), 1.0) / 128.0)
            noise = rng.normal(0.0, 1.0, img.shape) * sigma
        else:
            noise = rng.normal(0, spec.noise_sigma, img.shape)
        img = np.clip(np.rint(img.astype(np.float32) + noise), 0, 255).astype(np.uint8)
    if deg is not None and deg.jpeg_quality is not None:
        img = degrade.jpeg(img, deg.jpeg_quality)

    meta = {
        "name": spec.name, "width": intr.width, "height": intr.height,
        "camera": {"fx": intr.fx, "fy": intr.fy, "cx": intr.cx, "cy": intr.cy},
        "blur_sigma": spec.blur_sigma, "noise_sigma": spec.noise_sigma,
        "seed": spec.seed, "codes": truths,
    }
    if deg is not None:
        meta["degradations"] = {
            k: v for k, v in dataclasses.asdict(deg).items()
            if v is not None and not (k == "shot_noise" and not v)}
        for truth in truths:
            detect, decode, difficulty = scenarios.expectations(truth, deg)
            truth["expect_detect"] = detect
            truth["expect_decode"] = decode
            truth["difficulty"] = difficulty
    return img, meta


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--only", default=None,
                    help="only fixtures whose name starts with this prefix")
    args = ap.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    intr = camera.Intrinsics.default()
    specs = scenarios.build_all(args.seed)
    if args.only:
        specs = [s for s in specs if s.name.startswith(args.only)]
    for spec in specs:
        img, meta = render_fixture(spec, intr)
        ok, png = cv2.imencode(".png", img)
        assert ok
        (out / f"{spec.name}.png").write_bytes(png.tobytes())
        (out / f"{spec.name}.luma").write_bytes(img.tobytes())
        (out / f"{spec.name}.json").write_text(
            json.dumps(meta, indent=1, sort_keys=True) + "\n")
        print(f"wrote {spec.name} ({len(meta['codes'])} codes)")


if __name__ == "__main__":
    main()
