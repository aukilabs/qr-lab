//! Compile-time pins for every legacy `qrk_core` root import.

use qrk_core::{
    decode_bits, detect, detect_traced, detect_with, downscale_luma, downscaled_dims, find_finders,
    group_triplets, luma_from_rgba, scan, scan_robust, scan_robust_debug, scan_traced,
    BinarizeSpec, BitMatrix, DecodeFailure, DecodedCode, DecodedPayload, Detections,
    FinderCandidate, LumaError, LumaView, PerspectiveTransform, RobustCode, RobustDebug,
    RobustDetections, ScanConfig, ScanOptions, ScanSession, SessionConfig, StageClock,
    StageTimings, TileGrid, Trace, TripletCandidate, VariantKind, VariantRecord, VariantSnapshot,
};

#[test]
fn legacy_root_symbols_remain_importable() {
    let _ = decode_bits;
    let _ = detect;
    let _ = detect_traced;
    let _ = detect_with;
    let _ = downscale_luma;
    let _ = downscaled_dims;
    let _ = find_finders;
    let _ = group_triplets;
    let _ = luma_from_rgba;
    let _ = scan;
    let _ = scan_robust;
    let _ = scan_robust_debug;
    let _ = scan_traced;

    macro_rules! type_exists {
        ($($type:ty),+ $(,)?) => {
            $(let _ = std::mem::size_of::<Option<$type>>();)+
        };
    }
    type_exists!(
        BinarizeSpec,
        BitMatrix,
        DecodeFailure,
        DecodedCode,
        DecodedPayload,
        Detections,
        FinderCandidate,
        PerspectiveTransform,
        RobustCode,
        RobustDebug,
        RobustDetections,
        ScanConfig,
        ScanOptions,
        ScanSession,
        SessionConfig,
        StageClock,
        StageTimings,
        TileGrid,
        Trace,
        TripletCandidate,
        VariantKind,
        VariantRecord,
        VariantSnapshot,
    );
}

#[test]
fn legacy_luma_error_remains_exhaustive() {
    fn classify(error: LumaError) -> u8 {
        match error {
            LumaError::EmptyDimensions => 1,
            LumaError::StrideTooSmall => 2,
            LumaError::BufferTooSmall => 3,
        }
    }
    let data = [0u8; 4];
    let view: Result<LumaView<'_>, LumaError> = LumaView::new(&data, 2, 2, 2);
    assert!(view.is_ok());
    assert_eq!(classify(LumaError::StrideTooSmall), 2);
}
