#include "../include/qrk.h"
#include <stdlib.h>

int preprocess(const uint8_t *src, uint8_t *dst, uint32_t width, uint32_t height) {
    QrkOperatorContext *context = qrk_operator_context_create();
    if (context == NULL) return QRK_STATUS_PROCESSING_FAILED;

    QrkVanCittertConfigV1 config;
    int32_t status = qrk_van_cittert_config_v1_default(&config);
    if (status == QRK_STATUS_OK) {
        config.theta_radians = 0.0;
        config.blur_length = 5;
        status = qrk_van_cittert_luma_v1(
            context, src, width, height, width, dst, width, &config
        );
    }
    qrk_operator_context_destroy(context);
    return status;
}
