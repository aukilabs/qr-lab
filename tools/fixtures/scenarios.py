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
class MotionBlur:
    length_px: float
    angle_deg: float
    curve: float = 0.0


@dataclass(frozen=True)
class Shadow:
    strength: float          # illumination drop in the umbra, [0, 1]
    softness_px: float       # penumbra width (Gaussian sigma of field edge)
    angle_deg: float         # edge normal / band axis direction
    offset_xy: tuple         # point the edge / band / blob is anchored on
    shape: str = "half"      # {"half", "band", "blob"}
    size_px: float = 60.0    # band half-width / blob radius


@dataclass(frozen=True)
class IllumGradient:
    min_gain: float
    max_gain: float
    angle_deg: float


@dataclass(frozen=True)
class Glare:
    center_xy: tuple
    radius_px: float
    gain: float


@dataclass(frozen=True)
class Resolution:
    factor: int
    method: str              # {"area", "nearest"}


@dataclass(frozen=True)
class Occlusion:
    rect_xyxy: tuple         # image-space fill rectangle (x0, y0, x1, y1)
    gray: int
    target: str              # {"finder", "data"} — what the rect covers
    fraction: float          # fraction of the target region (finder) or of
                             # the whole symbol area (data)


@dataclass(frozen=True)
class Degradations:
    """Post-render image-space degradations. All None/off by default so the
    existing (pre-plan6) fixtures stay byte-identical. Application order is
    documented in degrade.py's module docstring."""
    motion_blur: object = None       # MotionBlur
    defocus_radius_px: float = None
    shadow: object = None            # Shadow
    illum_gradient: object = None    # IllumGradient
    glare: object = None             # Glare
    contrast_scale: float = None     # contrast_compress scale around 128
    resolution: object = None        # Resolution
    jpeg_quality: int = None
    shot_noise: bool = False         # signal-dependent noise variant
    occlusion: object = None         # Occlusion


@dataclass(frozen=True)
class FixtureSpec:
    name: str
    codes: tuple
    blur_sigma: float
    noise_sigma: float
    seed: int
    degradations: object = None      # Degradations or None (= legacy fixture)


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


# --- physics-derived expectations -------------------------------------
#
# Expectations are set from image-formation physics in MODULE units — never
# from running the scanner (fixtures verify, they don't tune). All thresholds
# below are conservative and justified inline. A fixture whose expectation
# proves wrong under the baseline scanner is evidence, not a knob.
#
# Constants:
_RENDER_CONTRAST = 210.0  # render.Levels: white 235 - black 25
_CONTRAST_FLOOR = 12.0    # qrk-core binarizer skip threshold (6 sigma of the
                          # fixture sensor-noise model, sigma = 2.0)
# Approximate fraction of codewords each ECC level can correct:
_ECC_CAPACITY = {"l": 0.07, "m": 0.15, "q": 0.25, "h": 0.30}
_EPS = 1e-6               # float guard for exactly-targeted module sizes


def expectations(truth, deg):
    """(expect_detect, expect_decode, difficulty 0..3) for one code.

    Rule summary (module size m = truth["module_size_px"], each active
    degradation contributes severity 1 = mild or 2 = strong; difficulty =
    0 / 1 / 2 / 3 for total severity 0 / 1 / 2 / >=3 — i.e. 1 = single mild,
    2 = single strong or double mild, 3 = combos/strong stacks):

    - module size (Nyquist): the finder run-ratio test tolerates ~+/-50%
      run-length error; at m < 2 px the +/-0.5 px sampling quantization
      exceeds that, so detect requires m >= 2.0. Decode requires m >= 2.5
      (conservative vs the ~3 px/module Nyquist-study floor) AND no other
      degradation when m < 3.0 (no margin left to absorb anything else).
    - motion blur, smear L/m: finder patterns (7-module structures) survive
      ~1-1.5 modules of smear; 1-module data cells corrupt first. decode
      needs L/m < 1.0, detect needs L/m < 1.5.
    - defocus radius r/m: a disc PSF of radius m/2 spans a full module and
      erases isolated cells -> decode needs r < m/2; the finder's 3-module
      core survives until r ~ 1.25 m -> detect needs r/m < 1.25.
    - shadow: residual contrast in the umbra = 210*(1-strength); detect
      needs residual >= CONTRAST_FLOOR (12). decode needs residual >= 24
      (2x floor) AND (strength < 0.5 OR softness_px >= m): a sharp
      (sub-module penumbra) edge at >= 50% depth bimodalizes threshold
      tiles it crosses; a penumbra wider than a module acts like a smooth
      gradient which local thresholding absorbs.
    - illumination gradient: smooth and monotonic — local thresholding is
      invariant to it as long as the darkest-end contrast clears the same
      bars: detect 210*min_gain >= 12, decode >= 24.
    - glare (hotspot centered on the code): center contrast scales by
      ~(1-gain); decode needs 210*(1-gain) >= 24. The hotspot is local
      (sigma = radius/2) so finders away from the center keep contrast ->
      detect stays true.
    - contrast_compress scale s: global remap, residual 210*s; detect
      needs >= 12, decode >= 24.
    - resolution factor f: effective module m/f. detect needs m/f >= 2.0
      (as above). decode: 'area' (box) needs m/f >= 2.5; 'nearest' needs
      m/f >= 3.0 — NN decimation adds +/-1-sample run jitter and can
      delete whole 1-module runs near Nyquist.
    - jpeg quality q: 8x8 blocking/ringing stays below module amplitude
      down to q ~ 25 at our module sizes (>= 6 px); below that block
      artifacts rival 1-module cells -> decode needs q >= 25. Finders (7+
      module structures, ~50 px) dwarf the 8 px block grid -> detect true.
    - occlusion: losing >= 25% of one finder breaks its 1:1:3:1:1 signature
      -> detect false. Data patch of fraction p corrupts ~p of codewords as
      a contiguous erasure burst; require p < 0.5 * ECC capacity (factor-2
      margin: burst erasures are worse than the random-error bound).
    - tilt >= 40 deg (combo fixtures only) counts as one mild severity: the
      minor-axis module pitch is roughly halved at 45 deg.
    """
    m = float(truth["module_size_px"])
    detect, decode = True, True
    severities = []

    other_ops = 0  # active degradation axes other than raw module size
    if deg is not None:
        other_ops = sum([
            deg.motion_blur is not None,
            deg.defocus_radius_px is not None,
            deg.shadow is not None,
            deg.illum_gradient is not None,
            deg.glare is not None,
            deg.contrast_scale is not None,
            deg.resolution is not None,
            deg.jpeg_quality is not None,
            bool(deg.shot_noise),
            deg.occlusion is not None,
        ])

    # module size (scene-space low resolution)
    if m + _EPS < 2.0:
        detect = False
        severities.append(2)
    elif m + _EPS < 2.5:
        decode = False
        severities.append(2)
    elif m + _EPS < 3.0:
        severities.append(1)
        if other_ops:
            decode = False

    if deg is not None:
        if deg.motion_blur is not None:
            ratio = deg.motion_blur.length_px / m
            if ratio >= 1.5:
                detect = False
            if ratio >= 1.0:
                decode = False
            severities.append(1 if ratio < 1.0 else 2)
        if deg.defocus_radius_px is not None:
            ratio = deg.defocus_radius_px / m
            if ratio >= 1.25:
                detect = False
            if ratio >= 0.5:
                decode = False
            severities.append(1 if ratio < 0.5 else 2)
        if deg.shadow is not None:
            s = deg.shadow
            residual = _RENDER_CONTRAST * (1.0 - s.strength)
            if residual < _CONTRAST_FLOOR:
                detect = False
            if residual < 2 * _CONTRAST_FLOOR or (
                    s.strength >= 0.5 and s.softness_px < m):
                decode = False
            severities.append(1 if s.strength < 0.6 else 2)
        if deg.illum_gradient is not None:
            residual = _RENDER_CONTRAST * deg.illum_gradient.min_gain
            if residual < _CONTRAST_FLOOR:
                detect = False
            if residual < 2 * _CONTRAST_FLOOR:
                decode = False
            severities.append(1 if residual >= 2 * _CONTRAST_FLOOR else 2)
        if deg.glare is not None:
            residual = _RENDER_CONTRAST * (1.0 - deg.glare.gain)
            if residual < 2 * _CONTRAST_FLOOR:
                decode = False
                severities.append(2)
            else:
                severities.append(1)
        if deg.contrast_scale is not None:
            residual = _RENDER_CONTRAST * deg.contrast_scale
            if residual < _CONTRAST_FLOOR:
                detect = False
            if residual < 2 * _CONTRAST_FLOOR:
                decode = False
            severities.append(1 if deg.contrast_scale >= 0.25 else 2)
        if deg.resolution is not None:
            m_eff = m / deg.resolution.factor
            if m_eff + _EPS < 2.0:
                detect = False
            floor = 2.5 if deg.resolution.method == "area" else 3.0
            if m_eff + _EPS < floor:
                decode = False
                severities.append(2)
            else:
                severities.append(1)
        if deg.jpeg_quality is not None:
            if deg.jpeg_quality < 25:
                decode = False
                severities.append(2)
            else:
                severities.append(1)
        if deg.shot_noise:
            severities.append(1)
        if deg.occlusion is not None:
            occ = deg.occlusion
            if occ.target == "finder":
                if occ.fraction >= 0.25:
                    detect = False
                    severities.append(2)
                else:
                    severities.append(1)
            else:  # data
                cap = _ECC_CAPACITY[truth["ecc"]]
                if occ.fraction >= 0.5 * cap:
                    decode = False
                    severities.append(2)
                else:
                    severities.append(1)

    if truth["tilt_deg"] >= 40.0:
        severities.append(1)

    if not detect:
        decode = False
    score = sum(severities)
    difficulty = 0 if score == 0 else 1 if score == 1 else \
        2 if score == 2 else 3
    return detect, decode, difficulty


def _targeted_code(rng, intr, name, target_px_per_module, *, version=1,
                   ecc="m", dist_range=(0.7, 1.1), tilt_max=10.0):
    """Code whose ground-truth module_size_px hits the target exactly
    (iterated correction against the same corner-derived formula
    generate.py records), like the ver_ family's size-from-fx logic."""
    import render
    n = version * 4 + 17
    dist = float(rng.uniform(*dist_range))
    tilt = float(rng.uniform(0.0, tilt_max))
    azimuth = float(rng.uniform(0, 360))
    inplane = float(rng.uniform(0, 360))
    point = (float(rng.uniform(400, intr.width - 400)),
             float(rng.uniform(250, intr.height - 250)))
    size = target_px_per_module * n * dist / intr.fx

    def make(size_m):
        return CodeSpec(
            payload=f"Q:{name}:0", version=version, ecc=ecc, mirrored=False,
            physical_size_m=size_m, distance_m=dist, tilt_deg=tilt,
            tilt_azimuth_deg=azimuth, inplane_deg=inplane, image_point=point)

    r, t = camera.make_pose(dist, tilt, azimuth, inplane, point, intr)
    for _ in range(8):
        corners = render.corners_px(intr, r, t, size)
        m = float(np.linalg.norm(corners[1] - corners[0]) / n)
        if abs(m - target_px_per_module) < 1e-9:
            break
        size *= target_px_per_module / m
    code = make(size)
    if not _fits(intr, code):
        raise RuntimeError(f"targeted code does not fit for {name}")
    return code


def _occl_rect(intr, code: CodeSpec, target, fraction):
    """Image-space occlusion rectangle from ground-truth geometry.

    target 'finder': a corner sub-square of the TL finder (7x7 modules at
    the symbol's TL corner) covering `fraction` of the finder's area.
    target 'data': a centered square covering `fraction` of the symbol area.
    Occlusion fixtures stay near-frontal (small tilt/in-plane) so the
    axis-aligned bbox of the projected square ~= the square itself.
    """
    n = code.version * 4 + 17
    half = code.physical_size_m / 2.0
    module_m = code.physical_size_m / n
    if target == "finder":
        side = 7.0 * module_m * np.sqrt(fraction)
        pts = np.array([[-half, -half], [-half + side, -half],
                        [-half + side, -half + side], [-half, -half + side]])
    elif target == "data":
        side = code.physical_size_m * np.sqrt(fraction) / 2.0
        pts = np.array([[-side, -side], [side, -side],
                        [side, side], [-side, side]])
    else:
        raise ValueError(f"unknown occlusion target {target!r}")
    r, t = camera.make_pose(code.distance_m, code.tilt_deg,
                            code.tilt_azimuth_deg, code.inplane_deg,
                            code.image_point, intr)
    px = camera.project(intr, r, t, pts)
    x0, y0 = np.floor(px.min(axis=0))
    x1, y1 = np.ceil(px.max(axis=0))
    return (int(x0), int(y0), int(x1) + 1, int(y1) + 1)


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

    # ------------------------------------------------------------------
    # Plan-6 robustness families. Appended strictly AFTER the original
    # matrix: the spec order and RNG derivation above are frozen so the
    # existing 81 fixtures stay byte-identical.
    # ------------------------------------------------------------------
    def addd(name, codes, deg, blur=0.6, noise=2.0):
        specs.append(FixtureSpec(
            name=name, codes=tuple(codes), blur_sigma=blur,
            noise_sigma=noise,
            seed=int(_rng_for(seed, name).integers(0, 2**31)),
            degradations=deg))

    # motion blur: length {4,8,12,16}px x angle {0,60}deg
    for i in range(8):
        name = f"mblur_{i:02d}"
        rng = _rng_for(seed, name)
        code = _sample_code(rng, intr, name, 0, version=2, ecc="m",
                            dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
        addd(name, [code], Degradations(motion_blur=MotionBlur(
            length_px=[4.0, 8.0, 12.0, 16.0][i % 4],
            angle_deg=[0.0, 60.0][i // 4])))

    # defocus: disc radius {2,4,6,8}px
    for i, radius in enumerate([2.0, 4.0, 6.0, 8.0]):
        name = f"defocus_{i:02d}"
        rng = _rng_for(seed, name)
        code = _sample_code(rng, intr, name, 0, version=2, ecc="m",
                            dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
        addd(name, [code], Degradations(defocus_radius_px=radius))

    # shadow: strength {0.45,0.75} x softness {2,32}px x shape {half,band},
    # edge anchored on the code's image point so it crosses the symbol.
    for i in range(8):
        name = f"shadow_{i:02d}"
        rng = _rng_for(seed, name)
        code = _sample_code(rng, intr, name, 0, version=2, ecc="m",
                            dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
        addd(name, [code], Degradations(shadow=Shadow(
            strength=[0.45, 0.75][i // 4],
            softness_px=[2.0, 32.0][(i // 2) % 2],
            angle_deg=float(rng.uniform(0, 360)),
            offset_xy=code.image_point,
            shape=["half", "band"][i % 2], size_px=60.0)))

    # illumination: 2 gradients (4:1), 2 glare-on-code, 2 contrast-compress
    for i in range(6):
        name = f"illum_{i:02d}"
        rng = _rng_for(seed, name)
        code = _sample_code(rng, intr, name, 0,
                            dist_range=(0.6, 1.1), tilt_range=(0.0, 15.0))
        if i < 2:
            deg = Degradations(illum_gradient=IllumGradient(
                min_gain=0.25, max_gain=1.0, angle_deg=[0.0, 90.0][i]))
        elif i < 4:
            deg = Degradations(glare=Glare(
                center_xy=code.image_point,
                radius_px=[90.0, 150.0][i - 2], gain=0.9))
        else:
            deg = Degradations(contrast_scale=[0.25, 0.15][i - 4])
        addd(name, [code], deg)

    # scene-space low resolution: small apparent module size, exact ground
    # truth, no resampling (2 fixtures per target px/module).
    for i in range(8):
        name = f"lowres_{i:02d}"
        rng = _rng_for(seed, name)
        code = _targeted_code(rng, intr, name,
                              [2.5, 2.0, 1.7, 1.4][i // 2])
        addd(name, [code], Degradations())

    # image-space resolution: down+upscale, factor {2,3} x {area,nearest},
    # from a ~7 px/module base.
    for i, (factor, method) in enumerate(
            [(2, "area"), (2, "nearest"), (3, "area"), (3, "nearest")]):
        name = f"res_{i:02d}"
        rng = _rng_for(seed, name)
        code = _targeted_code(rng, intr, name, 7.0, dist_range=(0.8, 1.0))
        addd(name, [code], Degradations(
            resolution=Resolution(factor=factor, method=method)))

    # jpeg: quality {40,25,15,10}
    for i, quality in enumerate([40, 25, 15, 10]):
        name = f"jpeg_{i:02d}"
        rng = _rng_for(seed, name)
        code = _sample_code(rng, intr, name, 0,
                            dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
        addd(name, [code], Degradations(jpeg_quality=quality))

    # occlusion: TL-finder corner clips and centered data patches;
    # near-frontal so the rect matches the projected region.
    occl_plan = [("finder", 0.10, "m"), ("finder", 0.25, "m"),
                 ("data", 0.05, "m"), ("data", 0.15, "m"),
                 ("data", 0.30, "m"), ("data", 0.05, "l")]
    for i, (target, fraction, ecc) in enumerate(occl_plan):
        name = f"occl_{i:02d}"
        rng = _rng_for(seed, name)
        code = _sample_code(rng, intr, name, 0, version=2, ecc=ecc,
                            dist_range=(0.6, 0.9), tilt_range=(0.0, 10.0),
                            inplane=float(rng.uniform(-8, 8)))
        rect = _occl_rect(intr, code, target, fraction)
        addd(name, [code], Degradations(occlusion=Occlusion(
            rect_xyxy=rect, gray=96, target=target, fraction=fraction)))

    # combos: the real-video failure profile (mixed axes; some decodable)
    for i in range(8):
        name = f"combo2_{i:02d}"
        rng = _rng_for(seed, name)
        blur, noise = 0.6, 2.0
        if i == 0:    # ~2 px/module + 6 px motion smear
            code = _targeted_code(rng, intr, name, 2.0)
            deg = Degradations(motion_blur=MotionBlur(
                6.0, float(rng.uniform(0, 180))))
        elif i == 1:  # sharp half-shadow crossing a 45-degree-tilted code
            code = _sample_code(rng, intr, name, 0, tilt=45.0,
                                dist_range=(0.5, 0.9))
            deg = Degradations(shadow=Shadow(
                0.6, 4.0, float(rng.uniform(0, 360)),
                code.image_point, "half", 60.0))
        elif i == 2:  # heavy jpeg on a low-contrast frame
            code = _sample_code(rng, intr, name, 0,
                                dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
            deg = Degradations(contrast_scale=0.3, jpeg_quality=20)
        elif i == 3:  # 8 px smear + soft shadow at ~2.5 px/module
            code = _targeted_code(rng, intr, name, 2.5)
            deg = Degradations(
                motion_blur=MotionBlur(8.0, float(rng.uniform(0, 180))),
                shadow=Shadow(0.5, 8.0, float(rng.uniform(0, 360)),
                              code.image_point, "half", 60.0))
        elif i == 4:  # mild defocus under a 4:1 gradient (decodable)
            code = _sample_code(rng, intr, name, 0,
                                dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
            deg = Degradations(defocus_radius_px=3.0,
                               illum_gradient=IllumGradient(
                                   0.25, 1.0, float(rng.uniform(0, 360))))
        elif i == 5:  # mild smear + moderate jpeg (decodable)
            code = _sample_code(rng, intr, name, 0,
                                dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
            deg = Degradations(motion_blur=MotionBlur(
                4.0, float(rng.uniform(0, 180))), jpeg_quality=40)
        elif i == 6:  # ~2.5 px/module + strong signal-dependent noise
            code = _targeted_code(rng, intr, name, 2.5)
            deg = Degradations(shot_noise=True)
            noise = 4.0
        else:         # curved smear + glare hotspot on the code
            code = _sample_code(rng, intr, name, 0,
                                dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
            deg = Degradations(
                motion_blur=MotionBlur(10.0, float(rng.uniform(0, 180)),
                                       curve=0.15),
                glare=Glare(code.image_point, 120.0, 0.9))
        addd(name, [code], deg, blur=blur, noise=noise)

    # mblur2: deep motion smears — the failure class the deblur tier exists
    # for. Length {20,28}px x angle {0,60}deg on ~8 px/module codes gives
    # 2.5-3.5 MODULE smears, past the ~1.5-module finder survival bound
    # (expectations() marks them detect=False): pure recovery-headroom
    # fixtures for the deconvolution rungs. Appended after every earlier
    # family so all existing RNG streams (per-name derived) and fixture
    # bytes are untouched.
    for i in range(4):
        name = f"mblur2_{i:02d}"
        rng = _rng_for(seed, name)
        code = _sample_code(rng, intr, name, 0, version=2, ecc="m",
                            dist_range=(0.6, 1.0), tilt_range=(0.0, 15.0))
        addd(name, [code], Degradations(motion_blur=MotionBlur(
            length_px=[20.0, 28.0][i % 2],
            angle_deg=[0.0, 60.0][i // 2])))
    # ... plus a lowres+blur combination: 2.5 px/module at the decode
    # resolution floor with an 8 px (3.2-module) smear — the hardest
    # real-video profile (small AND smeared).
    name = "mblur2_04"
    rng = _rng_for(seed, name)
    code = _targeted_code(rng, intr, name, 2.5)
    addd(name, [code], Degradations(motion_blur=MotionBlur(
        length_px=8.0, angle_deg=float(rng.uniform(0, 180)))))

    return specs
