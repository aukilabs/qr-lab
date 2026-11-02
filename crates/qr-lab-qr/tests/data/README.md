# Large finder regression frame

`large-finder-camera.png` is a lossless grayscale capture of a synthetic QR
at the iOS `scanLuma` boundary (1280×720, stride 1280), not a real photograph.
It contains binary bytes `00 FF 80 41 C3 28` (Base64 `AP+AQcMo`).

The input was a Segno-generated version-1 M QR, 8 pixels per module with a
4-module border (232×232). `serve-sim camera ... --file binary.png --mirror off`
resampled it into a 720×720 square centered in the camera frame. The fixture
preserves that resampling and black letterboxing. Its module-region bounds at
the 50% intensity crossing are x=378.5..900.5 and y=98.5..620.5 (±0.5 pixel).

At working resolution 1080×608, the original detector treated flat black
finder interiors as white. Only the upper two corners passed refinement;
the largest final corner error was 39.73 source pixels. The regression checks
the exact bytes, all four fit flags, and less than 2 pixels error per corner.
The generated scale/offset/polarity sweep in `large_finders.rs` independently
covers the same flat-tile mechanism without relying on this captured image.
