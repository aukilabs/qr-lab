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

## Real captures (`fixtures/real/`)

Real photos (PNG committed; `.luma`/`.json` are generated, gitignored — they
carry no ground truth and are NOT part of the golden gate suite). Regenerate
the scanner-readable pair for a photo with cv2 (grayscale read → raw bytes +
`{name,width,height,codes:[]}` JSON), then explore with:

    cargo run --release -p qrk-core --example scan_debug -- real/real_2 1280

Video workflow: source videos live locally in `fixtures/real/domain-data-mp4/`
(gitignored); extract a frame of interest with ffmpeg to a PNG (e.g.
`ffmpeg -i <video> -vf "select=eq(n\,167)" -vframes 1 video_f167.png`), commit
the PNG as the regression anchor, then explore it via the `decode_photo`
example or pin it in `decode_gate.rs`'s real-capture section.
