"""Deterministic scenario matrix for the golden fixture suite (spec §6)."""
import zlib
from dataclasses import dataclass

import numpy as np

import camera

MARGIN_PX = 40  # min distance of any corner from the image border


@dataclass(frozen=True)
class CodeSpec:
    payload: str
    version: int
    ecc: str
    mirrored: bool
    physical_size_m: float
    distance_m: float
    tilt_deg: float
    tilt_azimuth_deg: float
    inplane_deg: float
    image_point: tuple
    inverted: bool = False
    opaque_plate: bool = True


@dataclass(frozen=True)
class FixtureSpec:
    name: str
    codes: tuple
    blur_sigma: float
    noise_sigma: float
    seed: int


def _rng_for(master_seed, name):
    # crc32, not hash(): Python str hashing is salted per process and would
    # break cross-run determinism.
    return np.random.default_rng([master_seed, zlib.crc32(name.encode())])


def _fits(intr, code: CodeSpec) -> bool:
    import render
    r, t = camera.make_pose(code.distance_m, code.tilt_deg,
                            code.tilt_azimuth_deg, code.inplane_deg,
                            code.image_point, intr)
    quiet = render._plane_corners_m(
        code.physical_size_m, with_quiet=True,
        n_modules=code.version * 4 + 17)
    px, depth = camera.project(intr, r, t, quiet, return_depth=True)
    return bool(
        (depth > 0.1).all()
        and (px[:, 0] > MARGIN_PX).all()
        and (px[:, 0] < intr.width - MARGIN_PX).all()
        and (px[:, 1] > MARGIN_PX).all()
        and (px[:, 1] < intr.height - MARGIN_PX).all()
    )


def _sample_code(rng, intr, name, idx, *, version=1, ecc="m", mirrored=False,
                 size=0.15, dist_range=(0.5, 1.4), tilt_range=(0.0, 20.0),
                 inplane=None, tilt=None, inverted=False, opaque_plate=True):
    for _ in range(200):
        code = CodeSpec(
            payload=f"Q:{name}:{idx}",
            version=version, ecc=ecc, mirrored=mirrored,
            physical_size_m=size,
            distance_m=float(rng.uniform(*dist_range)),
            tilt_deg=float(rng.uniform(*tilt_range)) if tilt is None else tilt,
            tilt_azimuth_deg=float(rng.uniform(0, 360)),
            inplane_deg=(float(rng.uniform(0, 360))
                         if inplane is None else inplane),
            image_point=(float(rng.uniform(250, intr.width - 250)),
                         float(rng.uniform(180, intr.height - 180))),
            inverted=inverted, opaque_plate=opaque_plate,
        )
        if _fits(intr, code):
            return code
    raise RuntimeError(f"could not place code for {name}")


def _overlaps(intr, a: CodeSpec, b: CodeSpec) -> bool:
    import render

    def bbox(c):
        r, t = camera.make_pose(c.distance_m, c.tilt_deg, c.tilt_azimuth_deg,
                                c.inplane_deg, c.image_point, intr)
        quiet = render._plane_corners_m(
            c.physical_size_m, True, c.version * 4 + 17)
        px = camera.project(intr, r, t, quiet)
        return px[:, 0].min(), px[:, 1].min(), px[:, 0].max(), px[:, 1].max()

    ax0, ay0, ax1, ay1 = bbox(a)
    bx0, by0, bx1, by1 = bbox(b)
    return not (ax1 < bx0 or bx1 < ax0 or ay1 < by0 or by1 < ay0)


def build_all(seed: int):
    intr = camera.Intrinsics.default()
    specs = []

    def add(name, codes, blur, noise):
        specs.append(FixtureSpec(
            name=name, codes=tuple(codes), blur_sigma=blur,
            noise_sigma=noise,
            seed=int(_rng_for(seed, name).integers(0, 2**31))))

    for i in range(8):  # far: 1.5-2.0 m
        rng = _rng_for(seed, f"far_{i:02d}")
        add(f"far_{i:02d}",
            [_sample_code(rng, intr, f"far_{i:02d}", 0,
                          dist_range=(1.5, 2.0))], 0.8, 2.0)

    for i in range(8):  # near: < 1.5 m
        rng = _rng_for(seed, f"near_{i:02d}")
        add(f"near_{i:02d}",
            [_sample_code(rng, intr, f"near_{i:02d}", 0,
                          dist_range=(0.4, 1.45))], 0.6, 2.0)

    for i in range(12):  # in-plane rotation sweep, 30° steps
        rng = _rng_for(seed, f"rot_{i:02d}")
        add(f"rot_{i:02d}",
            [_sample_code(rng, intr, f"rot_{i:02d}", 0,
                          inplane=i * 30.0, tilt=0.0,
                          dist_range=(0.6, 1.2))], 0.6, 2.0)

    for i in range(8):  # 45° perspective tilt, azimuth swept
        rng = _rng_for(seed, f"tilt45_{i:02d}")
        add(f"tilt45_{i:02d}",
            [_sample_code(rng, intr, f"tilt45_{i:02d}", 0, tilt=45.0,
                          dist_range=(0.5, 1.1))], 0.6, 2.0)

    for i in range(8):  # multi: 1-4 codes/frame, cycling count
        name = f"multi_{i:02d}"
        rng = _rng_for(seed, name)
        want = (i % 4) + 1
        codes = []
        for k in range(want):
            for _ in range(200):
                c = _sample_code(rng, intr, name, k, size=0.10,
                                 dist_range=(0.7, 1.4),
                                 version=int(rng.choice([1, 2, 3])))
                if not any(_overlaps(intr, c, o) for o in codes):
                    codes.append(c)
                    break
            else:
                raise RuntimeError(f"could not place {want} codes in {name}")
        add(name, codes, 0.6, 2.0)

    versions = [1, 2, 3, 4, 5, 7, 10, 15, 20, 25, 30, 35, 40]
    for i, v in enumerate(versions):  # version sweep, ~5 px/module frontal
        name = f"ver_{i:02d}_v{v}"
        rng = _rng_for(seed, name)
        n = v * 4 + 17
        dist = 0.9
        # 5 px/module target, shrunk until the quiet-zone corners fit
        # in-frame — large versions (v30+) cannot reach 5 px/module in
        # 720p, especially under random in-plane rotation.
        size = 5.0 * n * dist / intr.fx
        payload = f"QRK:{name}:" + "x" * max(0, (v * v) // 2)
        tilt_deg = float(rng.uniform(0, 10))
        tilt_azimuth_deg = float(rng.uniform(0, 360))
        inplane_deg = float(rng.uniform(0, 360))

        def make(size_m):
            return CodeSpec(
                payload=payload, version=v,
                ecc=["l", "m", "q", "h"][i % 4], mirrored=False,
                physical_size_m=size_m, distance_m=dist,
                tilt_deg=tilt_deg,
                tilt_azimuth_deg=tilt_azimuth_deg,
                inplane_deg=inplane_deg,
                image_point=(intr.cx, intr.cy))

        code = make(size)
        for _ in range(100):
            if _fits(intr, code):
                break
            size *= 0.9
            code = make(size)
        else:
            raise RuntimeError(f"could not fit {name} in-frame")
        add(name, [code], 0.6, 2.0)

    for i in range(4):  # mirrored
        rng = _rng_for(seed, f"mirror_{i:02d}")
        add(f"mirror_{i:02d}",
            [_sample_code(rng, intr, f"mirror_{i:02d}", 0, mirrored=True,
                          dist_range=(0.6, 1.2))], 0.6, 2.0)

    # combo: far + 45° + rotated
    for i in range(4):
        rng = _rng_for(seed, f"combo_{i:02d}")
        add(f"combo_{i:02d}",
            [_sample_code(rng, intr, f"combo_{i:02d}", 0, tilt=45.0,
                          dist_range=(1.5, 1.8), size=0.18)], 0.8, 2.5)

    for i in range(6):  # inverted: light modules on dark plate
        rng = _rng_for(seed, f"inv_{i:02d}")
        add(f"inv_{i:02d}",
            [_sample_code(rng, intr, f"inv_{i:02d}", 0, ecc="l",
                          inverted=True,
                          dist_range=(0.5, 1.3))], 0.6, 2.0)

    for i in range(6):  # transparent: modules only, background quiet zone
        rng = _rng_for(seed, f"trans_{i:02d}")
        add(f"trans_{i:02d}",
            [_sample_code(rng, intr, f"trans_{i:02d}", 0, ecc="l",
                          opaque_plate=False,
                          dist_range=(0.5, 1.2))], 0.6, 2.0)

    for i in range(4):  # inverted + transparent
        rng = _rng_for(seed, f"invtrans_{i:02d}")
        add(f"invtrans_{i:02d}",
            [_sample_code(rng, intr, f"invtrans_{i:02d}", 0, ecc="l",
                          inverted=True,
                          opaque_plate=False, dist_range=(0.5, 1.2))], 0.6, 2.0)

    return specs
