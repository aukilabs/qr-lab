import json
from pathlib import Path

import cv2
import numpy as np
import pytest

SUITE = Path(__file__).resolve().parents[2] / "fixtures"
EASY_PREFIXES = ("near_", "rot_")


@pytest.fixture(scope="module")
def suite():
    metas = sorted(SUITE.glob("*.json"))
    if not metas:
        pytest.fail("fixture suite not generated — run generate.py first")
    return metas


def test_suite_is_complete(suite):
    names = [p.stem for p in suite]
    assert len(names) >= 81
    for stem in names:
        assert (SUITE / f"{stem}.png").exists()
        assert (SUITE / f"{stem}.luma").exists()


def test_opencv_decodes_easy_singles(suite):
    det = cv2.QRCodeDetector()
    checked = decoded = 0
    for meta_path in suite:
        meta = json.loads(meta_path.read_text())
        if not meta_path.stem.startswith(EASY_PREFIXES):
            continue
        if len(meta["codes"]) != 1 or meta["codes"][0]["mirrored"]:
            continue
        img = np.frombuffer(
            (SUITE / f"{meta_path.stem}.luma").read_bytes(), np.uint8,
        ).reshape(meta["height"], meta["width"])
        text, pts, _ = det.detectAndDecode(img)
        checked += 1
        if text == meta["codes"][0]["payload"]:
            decoded += 1
            # OpenCV corners include no subpixel guarantee; assert coarse
            # agreement (<8 px) with ground truth to catch gross errors.
            gt = np.array(meta["codes"][0]["corners_px"])
            got = pts.reshape(4, 2)
            best = min(
                np.abs(got - np.roll(gt, k, axis=0)).max() for k in range(4))
            assert best < 8.0, meta_path.stem
    assert checked >= 15
    # OpenCV won't get everything; require a solid majority.
    assert decoded >= int(checked * 0.7), f"{decoded}/{checked}"
