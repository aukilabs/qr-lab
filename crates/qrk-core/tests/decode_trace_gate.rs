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

/// Plan 5 Task 6: `MAX_DECODE_ROUNDS` (72) replaced the old flat
/// `MAX_DECODE_ATTEMPTS` (24) attempt cap with a round-counted budget (see
/// that constant's `consts.rs` doc for the full provenance). The budget
/// only exists to bound worst-case work on HOSTILE inputs — every golden
/// fixture, including the busiest multi-code scene in the suite
/// (`multi_07`: 8 triplets, 4 real codes, plenty of proximity-deduped/
/// rotation-retried noise around them), must stay comfortably under it, or
/// the "no behavioral change on any gate" claim for this task would be
/// false. `72` is inlined here (not imported) since `MAX_DECODE_ROUNDS` is
/// `pub(crate)`, not part of the public API this integration test compiles
/// against.
#[test]
fn round_budget_never_binds_on_the_golden_fixture_suite() {
    const MAX_DECODE_ROUNDS: usize = 72;
    let mut worst: (String, usize) = (String::new(), 0);
    for fixture in common::load_all() {
        let view = fixture.view();
        let mut trace = Trace::new();
        let _ = detect_traced(&view, &mut trace);
        let rounds_run: usize = trace.attempts.iter().map(|a| a.rounds.len().max(1)).sum();
        if rounds_run > worst.1 {
            worst = (fixture.name.clone(), rounds_run);
        }
        assert!(
            rounds_run < MAX_DECODE_ROUNDS,
            "{}: {rounds_run} rounds run — the round budget should never bind on a golden \
             fixture, only on hostile/adversarial input",
            fixture.name,
        );
    }
    // Sanity: the suite actually exercised a non-trivial amount of work
    // (catches an accidentally-empty fixture list silently passing).
    assert!(worst.1 > 0, "expected at least one fixture to run at least one round");
    println!("worst-case golden-suite rounds: {} ({} rounds)", worst.0, worst.1);
}
