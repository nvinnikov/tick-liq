//! End-to-end integration test (#21): fake pool tick stream → signal
//! engine → rebalance execution.
//!
//! Wires the real `SignalEngine` and `RebalanceEngine` together with
//! in-memory `TxSubmitter` and `RebalanceJournal` doubles, then drives a
//! synthetic tick stream through them. No live Solana, no database — the
//! point is to pin down the contract between the strategy and execution
//! layers so a refactor on either side can't silently break the wiring.
//!
//! What this test asserts:
//!   1. While the tick stream stays in-range, the signal engine emits
//!      `Hold` and the rebalance engine is never invoked.
//!   2. When the price walks out of the range and stays out for at
//!      least `min_out_of_range`, the signal flips to `Rebalance` with
//!      reason `OutOfRange`.
//!   3. The rebalance engine consumes that signal, records a `Pending`
//!      intent in the journal, asks the submitter to ship the plan, and
//!      marks the row `Completed` with the returned signature.
//!   4. The recorded intent's `(old_range, new_range, reason)` triple
//!      is the snapshot we'd get from a real Whirlpool builder — the
//!      same data that #31's instruction-set snapshot test will pin
//!      down at the lower layer.
//!   5. After `on_rebalance_executed` the signal engine resets and
//!      emits `Hold` on the next in-range tick.
//!
//! The submitter double records the full plan it received so the test
//! can also assert position id, target tick range, and stable reason
//! label round-trip end-to-end.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use solana_sdk::pubkey::Pubkey;

use tick_liq::execution::rebalance::{
    reason_label, RebalanceEngine, RebalanceIntent, RebalanceJournal, RebalancePlan, TickRange,
    TxSubmitter,
};
use tick_liq::strategy::pnl::PnlSnapshot;
use tick_liq::strategy::range::RangeRecommendation;
use tick_liq::strategy::signal::{
    MarketTick, RebalanceReason, RebalanceSignal, SignalConfig, SignalEngine,
};

// -----------------------------------------------------------------------------
// In-memory ports
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RecordedIntent {
    journal_id: i64,
    position_id: i64,
    old_range: (i32, i32),
    new_range: (i32, i32),
    reason: String,
    completed_with: Option<String>,
}

#[derive(Default)]
struct InMemoryJournal {
    rows: Mutex<Vec<RecordedIntent>>,
}

impl InMemoryJournal {
    fn snapshot(&self) -> Vec<RecordedIntent> {
        self.rows.lock().unwrap().clone()
    }
}

#[async_trait]
impl RebalanceJournal for InMemoryJournal {
    async fn record_intent(&self, intent: &RebalanceIntent<'_>) -> Result<i64> {
        let mut rows = self.rows.lock().unwrap();
        let id = (rows.len() as i64) + 1;
        rows.push(RecordedIntent {
            journal_id: id,
            position_id: intent.position_id,
            old_range: intent.old_range,
            new_range: intent.new_range,
            reason: intent.reason.to_string(),
            completed_with: None,
        });
        Ok(id)
    }

    async fn mark_completed(&self, journal_id: i64, tx_sig: &str) -> Result<()> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .iter_mut()
            .find(|r| r.journal_id == journal_id)
            .expect("journal_id should exist before mark_completed");
        row.completed_with = Some(tx_sig.to_string());
        Ok(())
    }
}

#[derive(Default)]
struct RecordingSubmitter {
    plans: Mutex<Vec<RebalancePlan>>,
    next_sig: Mutex<u64>,
}

impl RecordingSubmitter {
    fn snapshot(&self) -> Vec<RebalancePlan> {
        self.plans.lock().unwrap().clone()
    }
}

#[async_trait]
impl TxSubmitter for RecordingSubmitter {
    async fn submit_rebalance(&self, plan: &RebalancePlan) -> Result<String> {
        self.plans.lock().unwrap().push(plan.clone());
        let mut n = self.next_sig.lock().unwrap();
        *n += 1;
        Ok(format!("sig-{n}"))
    }
}

// -----------------------------------------------------------------------------
// The test
// -----------------------------------------------------------------------------

fn target_range() -> RangeRecommendation {
    // Caller-supplied target — the engine doesn't recompute it.
    RangeRecommendation {
        lower_tick: -512,
        upper_tick: 512,
        expected_capital_efficiency_ppm: 7_500_000,
    }
}

fn flat_pnl() -> PnlSnapshot {
    PnlSnapshot {
        fees_earned: 0.0,
        il_quote: 0.0,
        net: 0.0,
    }
}

fn tick(ts: u64, price: f64) -> MarketTick {
    MarketTick {
        timestamp_secs: ts,
        current_price: price,
        lower_price: 90.0,
        upper_price: 110.0,
        pnl: flat_pnl(),
        fees_earned_quote: 0.0,
        manual_request: false,
    }
}

#[tokio::test]
async fn monitor_signal_rebalance_pipeline() {
    // --- Wire up engines + doubles -----------------------------------------
    let cfg = SignalConfig {
        min_out_of_range: Duration::from_secs(60),
        ..SignalConfig::default()
    };
    let mut signal = SignalEngine::new(cfg);
    signal.set_target_range(target_range());

    let journal = Arc::new(InMemoryJournal::default());
    let submitter = Arc::new(RecordingSubmitter::default());
    let engine = RebalanceEngine::new(
        Arc::clone(&submitter) as Arc<dyn TxSubmitter>,
        Arc::clone(&journal) as Arc<dyn RebalanceJournal>,
    );

    let position_id = 42_i64;
    let position_mint = Pubkey::new_unique();
    let current_range = TickRange::new(-256, 256).unwrap();

    // Helper closure that mirrors what a real monitor loop would do.
    let make_plan = |reason: RebalanceReason, started_at_secs: u64| RebalancePlan {
        position_id,
        position_mint,
        current_range,
        target_range: target_range(),
        reason,
        started_at_secs,
    };

    // --- Phase 1: in-range stream → all Hold -------------------------------
    let in_range_stream: Vec<MarketTick> = (0..3).map(|i| tick(i * 10, 100.0)).collect();
    for t in &in_range_stream {
        let s = signal.on_tick(*t).expect("signal eval");
        assert_eq!(s, RebalanceSignal::Hold);
    }
    assert!(submitter.snapshot().is_empty(), "no submits during Hold");
    assert!(journal.snapshot().is_empty(), "no journal rows during Hold");

    // --- Phase 2: walk out of range ----------------------------------------
    // First out-of-range tick — engine starts the timer but holds.
    let s = signal.on_tick(tick(30, 130.0)).unwrap();
    assert_eq!(s, RebalanceSignal::Hold);
    // 30 s in: still under min_out_of_range.
    let s = signal.on_tick(tick(60, 130.0)).unwrap();
    assert_eq!(s, RebalanceSignal::Hold);
    // 60 s past first OOR tick: trigger fires.
    let trigger = signal.on_tick(tick(90, 130.0)).unwrap();
    let RebalanceSignal::Rebalance { reason, .. } = trigger else {
        panic!("expected Rebalance, got {trigger:?}");
    };
    assert_eq!(reason, RebalanceReason::OutOfRange);

    // --- Phase 3: hand the signal to the rebalance engine ------------------
    let outcome = engine
        .execute_signal(make_plan(reason, 90), trigger)
        .await
        .expect("execute_signal")
        .expect("Rebalance must produce an outcome");
    assert_eq!(outcome.executed_at_secs, 90);
    assert!(outcome.tx_sig.starts_with("sig-"));
    assert!(outcome.journal_id >= 1);

    // Submitter saw exactly one plan, with the signal-derived range and
    // a stable reason label round-tripping through TickRange.
    let plans = submitter.snapshot();
    assert_eq!(plans.len(), 1);
    let p = &plans[0];
    assert_eq!(p.position_id, position_id);
    assert_eq!(p.position_mint, position_mint);
    assert_eq!(p.current_range, current_range);
    assert_eq!(p.target_range.lower_tick, target_range().lower_tick);
    assert_eq!(p.target_range.upper_tick, target_range().upper_tick);
    assert_eq!(p.reason, RebalanceReason::OutOfRange);

    // Journal got a Pending row first then marked Completed. This is the
    // idempotency invariant under test — if the order ever flips, a real
    // submitter failure would lose the audit row.
    let rows = journal.snapshot();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.journal_id, outcome.journal_id);
    assert_eq!(row.position_id, position_id);
    assert_eq!(row.old_range, (-256, 256));
    assert_eq!(row.new_range, (-512, 512));
    assert_eq!(row.reason, reason_label(RebalanceReason::OutOfRange));
    assert_eq!(row.completed_with.as_deref(), Some(outcome.tx_sig.as_str()));

    // --- Phase 4: post-rebalance reset → next in-range tick is Hold --------
    signal.on_rebalance_executed(outcome.executed_at_secs);
    let s = signal.on_tick(tick(100, 100.0)).unwrap();
    assert_eq!(s, RebalanceSignal::Hold);
    // No additional submits or journal rows.
    assert_eq!(submitter.snapshot().len(), 1);
    assert_eq!(journal.snapshot().len(), 1);
}

#[tokio::test]
async fn hold_signal_does_not_touch_journal_or_submitter() {
    // Pure regression test for the Hold path through execute_signal: it
    // must short-circuit before any port is called. The pipeline test
    // above covers it implicitly via the in-range phase, but a dedicated
    // assertion makes the contract explicit.
    let journal = Arc::new(InMemoryJournal::default());
    let submitter = Arc::new(RecordingSubmitter::default());
    let engine = RebalanceEngine::new(
        Arc::clone(&submitter) as Arc<dyn TxSubmitter>,
        Arc::clone(&journal) as Arc<dyn RebalanceJournal>,
    );

    let plan = RebalancePlan {
        position_id: 1,
        position_mint: Pubkey::new_unique(),
        current_range: TickRange::new(-100, 100).unwrap(),
        target_range: target_range(),
        reason: RebalanceReason::Manual,
        started_at_secs: 0,
    };
    let outcome = engine
        .execute_signal(plan, RebalanceSignal::Hold)
        .await
        .unwrap();
    assert!(outcome.is_none());
    assert!(submitter.snapshot().is_empty());
    assert!(journal.snapshot().is_empty());
}
