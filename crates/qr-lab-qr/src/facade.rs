use crate::{
    scan_robust, LumaView, Rgb8View, RobustDetections, ScanConfig, ScanOptions, ScanSession,
    SessionConfig,
};

/// Complete-scanner configuration for the ergonomic [`Scanner`] facade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScannerConfig {
    /// Robust ladder configuration (which recovery rungs are enabled).
    pub scan: ScanConfig,
    /// Working-resolution cap and refinement flags.
    pub options: ScanOptions,
    /// When set, enables temporal video state (rung rotation + finder pool).
    pub session: Option<SessionConfig>,
}

impl ScannerConfig {
    /// Production-lean single-frame robust scanner.
    pub fn robust_fast() -> Self {
        Self {
            scan: ScanConfig::ROBUST_FAST,
            options: ScanOptions::default(),
            session: None,
        }
    }

    /// Full benchmark ladder with early exit disabled.
    pub fn robust_full() -> Self {
        Self {
            scan: ScanConfig::ROBUST_FULL_BENCHMARK,
            options: ScanOptions::default(),
            session: None,
        }
    }

    /// Enable temporal rung rotation and cross-frame finder pooling.
    pub fn temporal(mut self, session: SessionConfig) -> Self {
        self.session = Some(session);
        self
    }

    /// Replace the scan options (working-resolution cap, refinement, …).
    pub fn with_options(mut self, options: ScanOptions) -> Self {
        self.options = options;
        self
    }
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self::robust_fast()
    }
}

/// Stateful complete QR scanner.
///
/// In single-frame mode this is an ergonomic owner for scanner configuration.
/// With [`ScannerConfig::temporal`], it also retains the video session state
/// needed for rung rotation and cross-frame finder pooling.
pub struct Scanner {
    config: ScannerConfig,
    session: Option<ScanSession>,
    luma_scratch: Vec<u8>,
}

impl Scanner {
    /// Create a scanner from `config`.
    #[must_use]
    pub fn new(config: ScannerConfig) -> Self {
        let session = config
            .session
            .map(|session| ScanSession::new(config.scan, session));
        Self {
            config,
            session,
            luma_scratch: Vec::new(),
        }
    }

    /// Scan one grayscale frame and return robust detections with provenance.
    pub fn scan(&mut self, frame: &LumaView<'_>) -> RobustDetections {
        match &mut self.session {
            Some(session) => session.scan_frame(frame, &self.config.options),
            None => scan_robust(frame, &self.config.options, &self.config.scan),
        }
    }

    /// Convert and scan one packed RGB8 frame.
    ///
    /// The conversion uses BT.601 integer luminance. Its backing allocation is
    /// retained by the scanner and reused across frames.
    pub fn scan_rgb8(&mut self, frame: &Rgb8View<'_>) -> RobustDetections {
        frame.write_luma(&mut self.luma_scratch);
        let luma = LumaView::new(
            &self.luma_scratch,
            frame.width(),
            frame.height(),
            frame.width(),
        )
        .expect("RGB8 conversion always produces a valid luma layout");

        match &mut self.session {
            Some(session) => session.scan_frame(&luma, &self.config.options),
            None => scan_robust(&luma, &self.config.options, &self.config.scan),
        }
    }

    /// Clear temporal session state (no-op in single-frame mode).
    pub fn reset(&mut self) {
        if let Some(session) = &mut self.session {
            session.reset();
        }
    }

    /// Borrow the scanner configuration.
    pub const fn config(&self) -> &ScannerConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_matches_direct_single_frame_scan() {
        let pixels = vec![255; 32 * 24];
        let frame = LumaView::new(&pixels, 32, 24, 32).unwrap();
        let config = ScannerConfig::robust_fast();
        let expected = scan_robust(&frame, &config.options, &config.scan);
        let actual = Scanner::new(config).scan(&frame);
        assert_eq!(actual.codes.len(), expected.codes.len());
        assert_eq!(actual.finders.len(), expected.finders.len());
        assert_eq!(actual.triplets.len(), expected.triplets.len());
        assert_eq!(actual.variants.len(), expected.variants.len());
    }

    #[test]
    fn temporal_scanner_can_be_reset_and_reused() {
        let pixels = vec![255; 16 * 16];
        let frame = LumaView::new(&pixels, 16, 16, 16).unwrap();
        let config = ScannerConfig::robust_fast().temporal(SessionConfig::default());
        let mut scanner = Scanner::new(config);
        let first = scanner.scan(&frame);
        scanner.scan(&frame);
        scanner.reset();
        let replay = scanner.scan(&frame);
        assert_eq!(first.codes.len(), replay.codes.len());
        assert_eq!(first.finders.len(), replay.finders.len());
        assert_eq!(first.triplets.len(), replay.triplets.len());
    }

    #[test]
    fn rgb8_scanner_decodes_a_qr_from_padded_rows() {
        let payload = b"auki://portal/qr-lab-rgb8";
        let code = qrcode::QrCode::new(payload).unwrap();
        let module_count = code.width();
        let scale = 6;
        let quiet_zone = 4;
        let width = (module_count + 2 * quiet_zone) * scale;
        let height = width;
        let stride = width * 3 + 5;
        let mut pixels = vec![17; stride * height];
        for y in 0..height {
            for x in 0..width {
                let module_x = x / scale;
                let module_y = y / scale;
                let dark = module_x >= quiet_zone
                    && module_y >= quiet_zone
                    && module_x < quiet_zone + module_count
                    && module_y < quiet_zone + module_count
                    && code[(module_x - quiet_zone, module_y - quiet_zone)] == qrcode::Color::Dark;
                let value = if dark { 0 } else { 255 };
                pixels[y * stride + x * 3..y * stride + x * 3 + 3].fill(value);
            }
        }
        let frame = Rgb8View::new(&pixels, width, height, stride).unwrap();
        let result = Scanner::new(ScannerConfig::robust_fast()).scan_rgb8(&frame);
        assert!(result
            .codes
            .iter()
            .any(|detected| detected.code.payload_bytes == payload));
    }
}
