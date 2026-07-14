# qrkit

Umbrella Rust package for the complete QRKit QR scanner. It re-exports the
scanner API from `qrkit-qr` and exposes the focused crates as `image`,
`geometry`, and `imgproc` modules.

Use the focused crates directly when building a non-QR pipeline to avoid the QR
decoder dependency.
