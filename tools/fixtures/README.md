# Golden fixture generator

Renders QR codes through a pinhole camera with exact ground-truth corners.

    cd tools/fixtures
    python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
    .venv/bin/pytest            # unit tests
    .venv/bin/python generate.py --out ../../fixtures --seed 7

Regeneration is deterministic: same seed → byte-identical fixtures.
Ground truth per code: payload, version, ECC, mirrored, pose, and the 4
module-region corners (TL,TR,BR,BL in symbol space) in subpixel image px.

`module_size_px` is nominal (top-edge length / n); under tilt the true
module footprint varies across the symbol. `corners_px`, the camera
intrinsics, and `physical_size_m` are the authoritative ground truth
for pose — the pose angle fields (`tilt_deg`, `tilt_azimuth_deg`,
`inplane_deg`) are informational only.

## Regeneration environment

Fixtures were generated with Python 3.12.7 and the exact package
versions pinned in `requirements.txt`. Byte-determinism (same seed →
byte-identical output) is only guaranteed with those pinned versions —
newer numpy/opencv/segno releases can change rounding or encoding
behavior and produce different bytes even with the same seed.
