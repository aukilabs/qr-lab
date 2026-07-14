# qrkit-imgproc

Reusable CPU grayscale processing for QRKit and unrelated vision pipelines:

- resize and coordinate mapping
- tile-local and Sauvola thresholding
- rectangular grayscale morphology
- illumination/background division
- fixed-binomial sharpening
- structure-tensor direction and edge-rise blur estimation
- directional unsharp and Van Cittert line-PSF restoration

Allocating convenience functions are paired with caller-buffer forms and typed
workspaces for frame pipelines. This crate has no QR decoder or platform-binding
dependencies.
