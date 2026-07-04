//! Plan 4 Task 6: asserts `Trace` actually carries populated decode-stage
//! fields (`attempts`, `sample_regions`, `bits`; `alignment` is documented
//! as legitimately empty here — see below) after `detect_traced` on a real
//! fixture, not just that the fields compile.
//!
//! Gated behind `#![cfg(feature = "debug-trace")]` so `cargo test -p
//! qrk-core` (the default, feature-off build) skips this file entirely
//! rather than failing to compile against `Trace`'s zero-field no-feature
//! shape (see `src/trace.rs`'s doc comment). Run with:
//! `cargo test -p qrk-core --features debug-trace --test decode_trace_gate`.

#![cfg(feature = "debug-trace")]

mod common;

use qrk_core::{detect_traced, Trace};

#[test]
fn near_00_trace_has_populated_decode_fields() {
    let fixture = common::load("near_00");
    let view = fixture.view();
    let mut trace = Trace::new();
    let det = detect_traced(&view, &mut trace);

    assert_eq!(det.codes.len(), 1, "near_00 has one ground-truth code");
    assert_eq!(det.codes[0].payload, "Q:near_00:0");

    // attempts: one per attempted candidate this frame, every one carrying
    // per-round visibility (Task 6's carried-item fix).
    assert!(!trace.attempts.is_empty(), "expected at least one recorded attempt");
    assert!(
        trace.attempts.iter().any(|a| a.outcome == "decoded"),
        "expected a decoded attempt among {:?}",
        trace.attempts
    );
    assert!(
        trace.attempts.iter().all(|a| !a.rounds.is_empty()),
        "every attempt must record at least its first (parallelogram) round: {:?}",
        trace.attempts
    );

    // alignment: near_00's ground-truth code is v1 (dimension 21), which
    // has NO alignment patterns at all (ISO 18004) — so an empty vec here
    // is the correct, documented shape for this fixture, not a bug. A v7+
    // fixture would exercise non-empty `alignment`/`alignment_found` > 0;
    // deferred to the Task 7 QA pass (browser screenshot checks against a
    // higher-version fixture) rather than duplicated here.
    assert!(
        trace.alignment.is_empty(),
        "near_00 is v1 (no alignment patterns) — expected an empty alignment vec, got {:?}",
        trace.alignment
    );

    // sample_regions: v1 always samples through a single whole-grid region.
    assert_eq!(trace.sample_regions.len(), 1, "v1 candidates sample through one region");
    let region = &trace.sample_regions[0];
    assert_eq!(region.module_rect, [0, 0, 21, 21]);
    assert_eq!(region.quad.len(), 4);

    // bits: the last successfully decoded candidate's packed matrix.
    let bits = trace.bits.as_ref().expect("a successful decode must record its bit matrix");
    assert_eq!(bits.dim, 21);
    // words_per_row(21) == ceil(21/32) == 1, so 21 rows -> 21 words.
    assert_eq!(bits.words.len(), 21);
}
