# Golden fixture generator

Renders QR codes through a pinhole camera with exact ground-truth corners.

    cd tools/fixtures
    python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
    .venv/bin/pytest            # unit tests
    .venv/bin/python generate.py --out ../../fixtures --seed 7

Regeneration is deterministic: same seed → byte-identical fixtures.
Ground truth per code: payload, version, ECC, mirrored, pose, and the 4
module-region corners (TL,TR,BR,BL in symbol space) in subpixel image px.
