"""NumPy-first Python bindings for QRKit."""

from __future__ import annotations

from typing import Any

import numpy as np

from . import _native

__version__ = _native.__version__


def _gray8(image: Any) -> np.ndarray:
    array = np.asarray(image)
    if array.ndim != 2:
        raise ValueError(f"expected a 2-D grayscale image, got shape {array.shape!r}")
    if array.dtype != np.uint8:
        raise TypeError(f"expected dtype uint8, got {array.dtype}")
    if not array.flags.c_contiguous:
        array = np.ascontiguousarray(array)
    return array


def scan(
    image: Any,
    *,
    preset: str = "robust_fast",
    max_dimension: int = 1280,
    refine: bool = False,
) -> dict[str, Any]:
    """Scan one grayscale frame and return detections with source-pixel geometry."""
    return _native.scan(
        _gray8(image),
        preset=preset,
        max_dimension=max_dimension,
        refine=refine,
    )


class Scanner:
    """Reusable scanner, optionally with temporal video state."""

    def __init__(
        self,
        preset: str = "robust_fast",
        *,
        max_dimension: int = 1280,
        refine: bool = False,
        temporal: bool = False,
        rotation_period: int = 3,
        pool_ttl_frames: int = 4,
    ) -> None:
        self._inner = _native.Scanner(
            preset,
            max_dimension=max_dimension,
            refine=refine,
            temporal=temporal,
            rotation_period=rotation_period,
            pool_ttl_frames=pool_ttl_frames,
        )

    def scan(self, image: Any) -> dict[str, Any]:
        return self._inner.scan(_gray8(image))

    def reset(self) -> None:
        self._inner.reset()


def background_divide(
    image: Any,
    *,
    structuring_element: int = 31,
    target_luma: int = 200,
    denominator_floor: int = 8,
) -> np.ndarray:
    """Normalize smooth multiplicative illumination using background division."""
    return _native.background_divide(
        _gray8(image),
        structuring_element=structuring_element,
        target_luma=target_luma,
        denominator_floor=denominator_floor,
    )


def estimate_line_blur(
    image: Any,
    *,
    minimum_transitions: int = 8,
    minimum_amplitude: int = 24,
) -> dict[str, Any]:
    """Estimate motion-blur direction, anisotropy confidence, and line length."""
    return _native.estimate_line_blur(
        _gray8(image),
        minimum_transitions=minimum_transitions,
        minimum_amplitude=minimum_amplitude,
    )


def van_cittert(
    image: Any,
    theta_radians: float,
    blur_length: int,
    *,
    iterations: int = 3,
    relaxation: float = 1.0,
    border: str = "replicate",
    border_value: int = 0,
) -> np.ndarray:
    """Restore a known line point-spread function with Van Cittert iteration."""
    return _native.van_cittert(
        _gray8(image),
        theta_radians,
        blur_length,
        iterations=iterations,
        relaxation=relaxation,
        border=border,
        border_value=border_value,
    )


class ImageProcessor:
    """Reusable image-processing context which retains Rust scratch buffers."""

    def __init__(self) -> None:
        self._inner = _native.ImageProcessor()

    def background_divide(
        self,
        image: Any,
        *,
        structuring_element: int = 31,
        target_luma: int = 200,
        denominator_floor: int = 8,
    ) -> np.ndarray:
        return self._inner.background_divide(
            _gray8(image),
            structuring_element=structuring_element,
            target_luma=target_luma,
            denominator_floor=denominator_floor,
        )

    def estimate_line_blur(
        self,
        image: Any,
        *,
        minimum_transitions: int = 8,
        minimum_amplitude: int = 24,
    ) -> dict[str, Any]:
        return self._inner.estimate_line_blur(
            _gray8(image),
            minimum_transitions=minimum_transitions,
            minimum_amplitude=minimum_amplitude,
        )

    def van_cittert(
        self,
        image: Any,
        theta_radians: float,
        blur_length: int,
        *,
        iterations: int = 3,
        relaxation: float = 1.0,
        border: str = "replicate",
        border_value: int = 0,
    ) -> np.ndarray:
        return self._inner.van_cittert(
            _gray8(image),
            theta_radians,
            blur_length,
            iterations=iterations,
            relaxation=relaxation,
            border=border,
            border_value=border_value,
        )


__all__ = [
    "ImageProcessor",
    "Scanner",
    "__version__",
    "background_divide",
    "estimate_line_blur",
    "scan",
    "van_cittert",
]

