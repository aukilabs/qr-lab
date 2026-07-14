/**
 * qrk-ffi — C ABI for the QRKit scanner and reusable image operators.
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

/** Stable status codes returned by reusable operator functions. */
typedef enum QrkStatus {
    QRK_STATUS_OK = 0,
    QRK_STATUS_NULL_POINTER = 1,
    QRK_STATUS_INVALID_ARGUMENT = 2,
    QRK_STATUS_BUFFER_TOO_SMALL = 3,
    QRK_STATUS_PROCESSING_FAILED = 4
} QrkStatus;

/** Opaque reusable operator workspace. One context must not be used concurrently. */
typedef struct QrkOperatorContext QrkOperatorContext;

typedef struct QrkBackgroundDivideConfigV1 {
    uint32_t struct_size;
    uint32_t structuring_element;
    uint8_t target_luma;
    uint8_t denominator_floor;
    uint8_t reserved[2];
} QrkBackgroundDivideConfigV1;

typedef struct QrkVanCittertConfigV1 {
    uint32_t struct_size;
    uint32_t blur_length;
    uint32_t iterations;
    uint32_t border_mode; /* 0=replicate, 1=reflect, 2=constant */
    double theta_radians;
    double relaxation;
    uint8_t border_value; /* used only when border_mode == 2 */
    uint8_t reserved[7];
} QrkVanCittertConfigV1;

typedef struct QrkLineBlurEstimateV1 {
    uint32_t struct_size;
    uint32_t has_length;
    double theta_radians;
    double confidence;
    double length_px;
    int32_t raster_dx;
    int32_t raster_dy;
} QrkLineBlurEstimateV1;

QrkOperatorContext *qrk_operator_context_create(void);
void qrk_operator_context_destroy(QrkOperatorContext *context);

/** Pointer is owned by context and valid until its next operator call. */
const char *qrk_operator_last_error(const QrkOperatorContext *context);

int32_t qrk_background_divide_config_v1_default(
    QrkBackgroundDivideConfigV1 *out_config
);

int32_t qrk_van_cittert_config_v1_default(
    QrkVanCittertConfigV1 *out_config
);

int32_t qrk_background_divide_luma_v1(
    QrkOperatorContext *context,
    const uint8_t *src,
    uint32_t width,
    uint32_t height,
    uint32_t src_stride,
    uint8_t *dst,
    uint32_t dst_stride,
    const QrkBackgroundDivideConfigV1 *config
);

int32_t qrk_van_cittert_luma_v1(
    QrkOperatorContext *context,
    const uint8_t *src,
    uint32_t width,
    uint32_t height,
    uint32_t src_stride,
    uint8_t *dst,
    uint32_t dst_stride,
    const QrkVanCittertConfigV1 *config
);

int32_t qrk_estimate_line_blur_luma_v1(
    QrkOperatorContext *context,
    const uint8_t *src,
    uint32_t width,
    uint32_t height,
    uint32_t src_stride,
    QrkLineBlurEstimateV1 *out_estimate
);

#ifdef __cplusplus
}
#endif
