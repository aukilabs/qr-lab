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
                            ("mirror_", 4)]:
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
