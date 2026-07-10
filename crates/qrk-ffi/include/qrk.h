/**
 * qrk-ffi — C ABI for the qrk CPU QR scanner.
 *
 * All heap strings returned by this library are UTF-8, NUL-terminated, and
 * must be freed with `qrk_free_string`. Scan results are JSON objects; see
 * crates/qrk-ffi/README.md for the schema.
 */
#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/** Library version string (static; do not free). */
const char *qrk_version(void);

/**
 * Scan an 8-bit grayscale frame.
 *
 * @param luma     tightly packed or strided Y plane
 * @param width    width in pixels (> 0)
 * @param height   height in pixels (> 0)
 * @param stride   row stride in bytes (>= width); pass width for tightly packed
 * @param max_dim  working-resolution cap on the longest side; 0 = no cap
 * @param refine   non-zero enables subpixel corner refinement
 * @return         heap JSON string on success, or NULL on invalid input.
 *                 Free with `qrk_free_string`.
 */
char *qrk_scan_luma(
    const uint8_t *luma,
    uint32_t width,
    uint32_t height,
    uint32_t stride,
    uint32_t max_dim,
    int32_t refine
);

/** Free a string returned by `qrk_scan_luma` (NULL-safe). */
void qrk_free_string(char *ptr);

#ifdef __cplusplus
}
#endif
