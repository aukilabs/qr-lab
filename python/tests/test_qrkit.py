from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest

import auki_qrkit


ROOT = Path(__file__).resolve().parents[2]


def test_robust_scanner_decodes_committed_fixture() -> None:
    metadata = json.loads((ROOT / "fixtures/near_00.json").read_text())
    image = np.fromfile(ROOT / "fixtures/near_00.luma", dtype=np.uint8).reshape(
        metadata["height"], metadata["width"]
    )

    result = auki_qrkit.scan(image, preset="robust_fast", refine=True)
    payloads = {entry["code"]["payload"] for entry in result["codes"]}

    assert metadata["codes"][0]["payload"] in payloads


def test_temporal_scanner_resets_and_accepts_non_contiguous_input() -> None:
    image = np.full((24, 32), 255, dtype=np.uint8)[:, ::-1]
    scanner = auki_qrkit.Scanner(temporal=True)
    assert scanner.scan(image)["codes"] == []
    scanner.reset()
    assert scanner.scan(image)["codes"] == []


def test_operator_shapes_dtypes_and_workspace_reuse() -> None:
    image = np.tile(np.arange(64, dtype=np.uint8), (48, 1))
    processor = auki_qrkit.ImageProcessor()

    normalized = processor.background_divide(image, structuring_element=15)
    estimate = processor.estimate_line_blur(image)
    restored = processor.van_cittert(image, 0.0, 3)
    restored_again = processor.van_cittert(image, 0.0, 3)

    assert normalized.shape == image.shape
    assert normalized.dtype == np.uint8
    assert set(estimate) == {
        "theta_radians",
        "confidence",
        "raster_direction",
        "blur_length",
    }
    assert restored.dtype == np.uint8
    np.testing.assert_array_equal(restored, restored_again)


def test_constant_image_is_invariant_under_matching_constant_border() -> None:
    image = np.full((7, 9), 120, dtype=np.uint8)
    restored = auki_qrkit.van_cittert(
        image, 0.0, 5, border="constant", border_value=120
    )
    np.testing.assert_array_equal(restored, image)


def test_invalid_images_and_configuration_raise_python_exceptions() -> None:
    with pytest.raises(ValueError, match="2-D"):
        auki_qrkit.scan(np.zeros((2, 3, 4), dtype=np.uint8))
    with pytest.raises(TypeError, match="uint8"):
        auki_qrkit.scan(np.zeros((3, 4), dtype=np.float32))
    with pytest.raises(ValueError, match="unknown preset"):
        auki_qrkit.scan(np.zeros((3, 4), dtype=np.uint8), preset="unknown")
    with pytest.raises(RuntimeError, match="configuration"):
        auki_qrkit.van_cittert(np.zeros((3, 4), dtype=np.uint8), 0.0, 4)
