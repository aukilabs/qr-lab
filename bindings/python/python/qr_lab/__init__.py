"""NumPy-first Python bindings for QR Lab.

Install with ``pip install qr-lab``. Public entry points:

* :func:`scan` — one-shot frame scan
* :class:`Scanner` — reusable / temporal scanner
* :func:`background_divide`, :func:`estimate_line_blur`, :func:`van_cittert`
* :class:`ImageProcessor` — operators with retained scratch buffers

All image inputs must be 2-D ``uint8`` grayscale arrays (``H × W``).
"""

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
    """Scan one grayscale frame.

    Parameters
    ----------
    image:
        2-D ``uint8`` grayscale array shaped ``(height, width)``.
    preset:
        ``"robust_fast"`` (default, production) or ``"robust_full"`` (full ladder).
    max_dimension:
        Cap on the longest working-resolution side; ``0`` disables the cap.
    refine:
        When true, run subpixel corner refinement on decoded codes.

    Returns
    -------
    dict
        Detection payload including ``codes`` with ``payload`` and
        ``corners_source`` (TL/TR/BR/BL in source pixels).
    """
    return _native.scan(
        _gray8(image),
        preset=preset,
        max_dimension=max_dimension,
        refine=refine,
    )


class Scanner:
    """Reusable scanner, optionally with temporal video state.

    When ``temporal=True``, the scanner retains rung-rotation and cross-frame
    finder-pool state across :meth:`scan` calls. Call :meth:`reset` after a
    scene cut or camera switch.
    """

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
        """Create a scanner.

        Parameters
        ----------
        preset:
            ``"robust_fast"`` or ``"robust_full"``.
        max_dimension:
            Working-resolution long-side cap; ``0`` disables the cap.
        refine:
            Enable subpixel corner refinement.
        temporal:
            Enable video session state.
        rotation_period:
            Frames between temporal ladder rung rotations (temporal mode).
        pool_ttl_frames:
            Lifetime of pooled finder candidates in frames (temporal mode).
        """
        self._inner = _native.Scanner(
            preset,
            max_dimension=max_dimension,
            refine=refine,
            temporal=temporal,
            rotation_period=rotation_period,
            pool_ttl_frames=pool_ttl_frames,
        )

    def scan(self, image: Any) -> dict[str, Any]:
        """Scan one frame; see :func:`scan` for the return shape."""
        return self._inner.scan(_gray8(image))

    def reset(self) -> None:
        """Clear temporal session state (no-op when ``temporal=False``)."""
        self._inner.reset()


def background_divide(
    image: Any,
    *,
    structuring_element: int = 31,
    target_luma: int = 200,
    denominator_floor: int = 8,
) -> np.ndarray:
    """Normalize smooth multiplicative illumination using background division.

    Returns a new ``uint8`` array with the same shape as ``image``.
    """
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
    """Estimate motion-blur direction, anisotropy confidence, and line length.

    Returns a dict with ``theta_radians``, ``confidence``, and optional
    ``blur_length`` (pixels).
    """
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
    """Restore a known line point-spread function with Van Cittert iteration.

    ``blur_length`` must be odd and at least 3. ``border`` is one of
    ``"replicate"``, ``"reflect"``, or ``"constant"``.
    """
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
    """Reusable image-processing context which retains Rust scratch buffers.

    Prefer this over the free functions when applying operators to many frames.
    """

    def __init__(self) -> None:
        """Allocate an empty operator workspace."""
        self._inner = _native.ImageProcessor()

    def background_divide(
        self,
        image: Any,
        *,
        structuring_element: int = 31,
        target_luma: int = 200,
        denominator_floor: int = 8,
    ) -> np.ndarray:
        """See :func:`background_divide`."""
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
        """See :func:`estimate_line_blur`."""
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
        """See :func:`van_cittert`."""
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
