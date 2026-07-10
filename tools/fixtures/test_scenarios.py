import json
import subprocess
import sys
from pathlib import Path

import numpy as np
import scenarios


def test_matrix_covers_spec_scenarios():
    specs = scenarios.build_all(seed=7)
    names = [s.name for s in specs]
    for prefix, minimum in [("far_", 8), ("near_", 8), ("rot_", 12),
                            ("tilt45_", 8), ("multi_", 8), ("ver_", 13),
                            ("mirror_", 4), ("inv_", 6), ("trans_", 6),
                            ("invtrans_", 4),
                            ("mblur_", 8), ("defocus_", 4), ("shadow_", 8),
                            ("illum_", 6), ("lowres_", 8), ("res_", 4),
                            ("jpeg_", 4), ("occl_", 6), ("combo2_", 8)]:
        assert sum(n.startswith(prefix) for n in names) >= minimum, prefix


def test_matrix_is_deterministic():
    a = scenarios.build_all(seed=7)
    b = scenarios.build_all(seed=7)
    assert a == b
    c = scenarios.build_all(seed=8)
    assert a != c


def test_far_and_near_distances_respect_spec():
    specs = scenarios.build_all(seed=7)
    for s in specs:
        for code in s.codes:
            if s.name.startswith("far_"):
                assert 1.5 <= code.distance_m <= 2.0
            if s.name.startswith("near_"):
                assert code.distance_m < 1.5


def test_multi_has_1_to_4_codes_and_unique_payloads():
    specs = [s for s in scenarios.build_all(seed=7)
             if s.name.startswith("multi_")]
    counts = {len(s.codes) for s in specs}
    assert counts == {1, 2, 3, 4}
    for s in specs:
        payloads = [c.payload for c in s.codes]
        assert len(set(payloads)) == len(payloads)


def test_version_sweep_scales_physical_size_for_module_px():
    specs = [s for s in scenarios.build_all(seed=7)
             if s.name.startswith("ver_")]
    versions = sorted(c.version for s in specs for c in s.codes)
    assert versions[0] == 1 and versions[-1] == 40


def test_every_code_fits_in_frame():
    import camera
    intr = camera.Intrinsics.default()
    for s in scenarios.build_all(seed=7):
        for code in s.codes:
            assert scenarios._fits(intr, code), (s.name, code.payload)


def test_generate_cli_writes_fixture_triplet(tmp_path):
    subprocess.run(
        [sys.executable, "generate.py", "--out", str(tmp_path),
         "--seed", "7", "--only", "near_00"],
        check=True, cwd=Path(__file__).parent)
    meta = json.loads((tmp_path / "near_00.json").read_text())
    assert meta["width"] == 1280 and meta["height"] == 720
    luma = (tmp_path / "near_00.luma").read_bytes()
    assert len(luma) == 1280 * 720
    img = np.frombuffer(luma, np.uint8).reshape(720, 1280)
    for code in meta["codes"]:
        c = np.array(code["corners_px"])
        assert (c[:, 0] > 0).all() and (c[:, 0] < 1279).all()
        assert (c[:, 1] > 0).all() and (c[:, 1] < 719).all()
        # Dark ink just inside the TL corner (0.35 modules per axis —
        # inside the finder's dark outer ring), light quiet zone just
        # outside.
        n = {1: 21}.get(code["version"], code["version"] * 4 + 17)
        mod = code["module_size_px"]
        tl = c[0]
        inward = (c[2] - c[0]) / np.linalg.norm(c[2] - c[0])
        pin = (tl + inward * mod * 0.5).astype(int)
        pout = (tl - inward * mod * 0.5).astype(int)
        assert img[pin[1], pin[0]] < 110
        assert img[pout[1], pout[0]] > 150


def test_generate_cli_is_byte_deterministic(tmp_path):
    for d in ("a", "b"):
        subprocess.run(
            [sys.executable, "generate.py", "--out", str(tmp_path / d),
             "--seed", "7", "--only", "rot_00"],
            check=True, cwd=Path(__file__).parent)
    for ext in (".png", ".luma", ".json"):
        fa = (tmp_path / "a" / f"rot_00{ext}").read_bytes()
        fb = (tmp_path / "b" / f"rot_00{ext}").read_bytes()
        assert fa == fb, ext


# --- plan6 robustness families -------------------------------------------

def test_legacy_specs_have_no_degradations():
    for s in scenarios.build_all(seed=7):
        legacy = not s.name.startswith(
            ("mblur_", "mblur2_", "defocus_", "shadow_", "illum_", "lowres_",
             "res_", "jpeg_", "occl_", "combo2_"))
        if legacy:
            assert s.degradations is None, s.name
        else:
            assert s.degradations is not None, s.name


def test_lowres_hits_target_module_size():
    import camera
    import render
    intr = camera.Intrinsics.default()
    targets = {0: 2.5, 1: 2.5, 2: 2.0, 3: 2.0, 4: 1.7, 5: 1.7,
               6: 1.4, 7: 1.4}
    specs = {s.name: s for s in scenarios.build_all(seed=7)}
    for i, target in targets.items():
        code = specs[f"lowres_{i:02d}"].codes[0]
        r, t = camera.make_pose(code.distance_m, code.tilt_deg,
                                code.tilt_azimuth_deg, code.inplane_deg,
                                code.image_point, intr)
        c = render.corners_px(intr, r, t, code.physical_size_m)
        n = code.version * 4 + 17
        m = float(np.linalg.norm(c[1] - c[0]) / n)
        assert abs(m - target) < 1e-6, (i, m)


def test_occlusion_rect_covers_expected_region():
    specs = {s.name: s for s in scenarios.build_all(seed=7)}
    for name, spec in specs.items():
        if not name.startswith("occl_"):
            continue
        occ = spec.degradations.occlusion
        x0, y0, x1, y1 = occ.rect_xyxy
        assert x0 < x1 and y0 < y1
        assert 0 <= x0 and x1 <= 1280 and 0 <= y0 and y1 <= 720


def test_expectations_rules():
    def truth(m, ecc="m", tilt=5.0):
        return {"module_size_px": m, "ecc": ecc, "tilt_deg": tilt}

    D = scenarios.Degradations
    # no degradation, comfortable module size: everything easy
    assert scenarios.expectations(truth(6.0), D()) == (True, True, 0)
    # scene-space low resolution ladder
    assert scenarios.expectations(truth(2.5), D()) == (True, True, 1)
    assert scenarios.expectations(truth(2.0), D()) == (True, False, 2)
    assert scenarios.expectations(truth(1.4), D()) == (False, False, 2)
    # m in [2.5, 3.0) decodes only with no other degradation
    d, dec, _ = scenarios.expectations(truth(2.5), D(shot_noise=True))
    assert d is True and dec is False
    # motion blur in module units
    mb = scenarios.MotionBlur
    assert scenarios.expectations(
        truth(8.0), D(motion_blur=mb(4.0, 0.0))) == (True, True, 1)
    assert scenarios.expectations(
        truth(8.0), D(motion_blur=mb(8.0, 0.0)))[:2] == (True, False)
    assert scenarios.expectations(
        truth(8.0), D(motion_blur=mb(16.0, 0.0)))[:2] == (False, False)
    # defocus: half-module radius kills decode, 1.25 m kills detect
    assert scenarios.expectations(
        truth(8.0), D(defocus_radius_px=2.0)) == (True, True, 1)
    assert scenarios.expectations(
        truth(8.0), D(defocus_radius_px=6.0))[:2] == (True, False)
    assert scenarios.expectations(
        truth(4.0), D(defocus_radius_px=6.0))[:2] == (False, False)
    # shadow: sharp+strong kills decode, soft or mild survives
    sh = scenarios.Shadow
    assert scenarios.expectations(
        truth(7.0), D(shadow=sh(0.45, 2.0, 0.0, (0, 0))))[:2] == (True, True)
    assert scenarios.expectations(
        truth(7.0), D(shadow=sh(0.75, 2.0, 0.0, (0, 0))))[:2] == (True, False)
    assert scenarios.expectations(
        truth(7.0), D(shadow=sh(0.75, 32.0, 0.0, (0, 0))))[:2] == (True, True)
    # occlusion: >=25% of a finder breaks detection; data vs ECC capacity
    oc = scenarios.Occlusion
    assert scenarios.expectations(
        truth(7.0), D(occlusion=oc((0, 0, 1, 1), 96, "finder", 0.25))
    )[:2] == (False, False)
    assert scenarios.expectations(
        truth(7.0), D(occlusion=oc((0, 0, 1, 1), 96, "finder", 0.10))
    )[:2] == (True, True)
    assert scenarios.expectations(
        truth(7.0), D(occlusion=oc((0, 0, 1, 1), 96, "data", 0.05))
    )[:2] == (True, True)
    assert scenarios.expectations(
        truth(7.0, ecc="l"), D(occlusion=oc((0, 0, 1, 1), 96, "data", 0.05))
    )[:2] == (True, False)
    # jpeg: quality floor at 25
    assert scenarios.expectations(
        truth(7.0), D(jpeg_quality=25))[:2] == (True, True)
    assert scenarios.expectations(
        truth(7.0), D(jpeg_quality=15))[:2] == (True, False)
    # combos escalate difficulty to 3
    assert scenarios.expectations(
        truth(2.0), D(motion_blur=mb(6.0, 0.0))) == (False, False, 3)


def test_new_family_payload_convention():
    for s in scenarios.build_all(seed=7):
        if s.name.startswith(("mblur_", "defocus_", "shadow_", "illum_",
                              "lowres_", "res_", "jpeg_", "occl_",
                              "combo2_")):
            assert [c.payload for c in s.codes] == [f"Q:{s.name}:0"], s.name
