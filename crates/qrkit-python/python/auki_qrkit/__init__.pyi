from typing import Any, Literal, TypedDict

import numpy as np
from numpy.typing import ArrayLike, NDArray

Preset = Literal["baseline", "robust_fast", "robust_full"]
BorderMode = Literal["replicate", "reflect", "constant"]

class BlurEstimate(TypedDict):
    theta_radians: float
    confidence: float
    raster_direction: Literal["horizontal", "down_right", "vertical", "down_left"]
    blur_length: float | None

__version__: str

def scan(
    image: ArrayLike,
    *,
    preset: Preset = ...,
    max_dimension: int = ...,
    refine: bool = ...,
) -> dict[str, Any]: ...

class Scanner:
    def __init__(
        self,
        preset: Preset = ...,
        *,
        max_dimension: int = ...,
        refine: bool = ...,
        temporal: bool = ...,
        rotation_period: int = ...,
        pool_ttl_frames: int = ...,
    ) -> None: ...
    def scan(self, image: ArrayLike) -> dict[str, Any]: ...
    def reset(self) -> None: ...

def background_divide(
    image: ArrayLike,
    *,
    structuring_element: int = ...,
    target_luma: int = ...,
    denominator_floor: int = ...,
) -> NDArray[np.uint8]: ...

def estimate_line_blur(
    image: ArrayLike,
    *,
    minimum_transitions: int = ...,
    minimum_amplitude: int = ...,
) -> BlurEstimate: ...

def van_cittert(
    image: ArrayLike,
    theta_radians: float,
    blur_length: int,
    *,
    iterations: int = ...,
    relaxation: float = ...,
    border: BorderMode = ...,
    border_value: int = ...,
) -> NDArray[np.uint8]: ...

class ImageProcessor:
    def __init__(self) -> None: ...
    def background_divide(
        self,
        image: ArrayLike,
        *,
        structuring_element: int = ...,
        target_luma: int = ...,
        denominator_floor: int = ...,
    ) -> NDArray[np.uint8]: ...
    def estimate_line_blur(
        self,
        image: ArrayLike,
        *,
        minimum_transitions: int = ...,
        minimum_amplitude: int = ...,
    ) -> BlurEstimate: ...
    def van_cittert(
        self,
        image: ArrayLike,
        theta_radians: float,
        blur_length: int,
        *,
        iterations: int = ...,
        relaxation: float = ...,
        border: BorderMode = ...,
        border_value: int = ...,
    ) -> NDArray[np.uint8]: ...

