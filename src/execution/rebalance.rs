//! Rebalance execution engine: close → collect fees → open new position.
//!
//! The engine is the consumer of [`SignalEngine::on_tick`]: when the signal
//! engine emits `RebalanceSignal::Rebalance { reason, target_range }`, this
//! module translates that into the chain-side sequence
//!
//!   1. close the existing position (decrease liquidity to 0, close account)
//!   2. collect outstanding fees + rewards
//!   3. open a new position at `target_range`
//!   4. increase liquidity with the collected balances
//!
//! ## What this module does NOT do
//!
//! - It does not decide **whether** to rebalance; that's the signal
//!   engine's job. The engine is "execute this plan."
//! - It does not sign or submit transactions directly; those go through
//!   a [`TxSubmitter`] which is a trait so the real Solana client
//!   (task #15) and an in-memory test double can share the same flow.
//! - It does not call the range optimizer; `target_range` arrives in the
//!   signal already aligned to tick spacing.
//! - It does not swap tokens when the collected balances don't match the
//!   target range's required ratio — that's a TODO called out on
//!   [`RebalancePlan`] and will be a follow-up task.
//!
//! ## Idempotency and the journal
//!
//! If the close instruction succeeds but the open instruction fails midway
//! through, the on-chain state is "no position, token balances in wallet"
//! and the strategy must be able to recover on the next tick. We enforce
//! this by writing the rebalance **intent** to the [`RebalanceJournal`]
//! *before* calling the submitter, and updating the same row to
//! `Completed` afterwards. A row stuck in `Pending` at startup is the
//! recovery signal for whoever owns the replay path (follow-up task).
//!
//! ## Observability
//!
//! Every execute call is wrapped in `observability::rebalance_span` with
//! a **stable** reason string (see [`reason_label`]) — dashboards and log
//! queries key on this string, so it is converted via an explicit match
//! rather than `Debug` so a compiler-level refactor can't silently rename
//! dashboard fields.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use solana_sdk::pubkey::Pubkey;
use tracing::Instrument;

use crate::observability::rebalance_span;
use crate::strategy::range::RangeRecommendation;
use crate::strategy::signal::{RebalanceReason, RebalanceSignal};

// -----------------------------------------------------------------------------
// Plan and outcome
// -----------------------------------------------------------------------------

/// Inputs the engine needs to execute a rebalance against a specific
/// position.
#[derive(Debug, Clone)]
pub struct RebalancePlan {
    /// Database id of the position being rebalanced. Used to thread the
    /// journal row through close/open.
    pub position_id: i64,
    /// On-chain position NFT mint, used for log/span fields.
    pub position_mint: Pubkey,
    /// Currently-active tick range. Recorded in the journal as
    /// `old_range` before any on-chain action.
    pub current_range: TickRange,
    /// Target range that the engine will open. Supplied by the signal.
    pub target_range: RangeRecommendation,
    /// Reason the rebalance was triggered.
    pub reason: RebalanceReason,
    /// Wall-clock seconds at which execution starts. Threaded back into
    /// `SignalEngine::on_rebalance_executed` so the signal state machine
    /// resets its out-of-range timer consistently.
    pub started_at_secs: u64,
    // TODO(follow-up): swap_required: Option<SwapQuote>  — when the
    // collected balances don't fit the target range's token ratio.
}

/// A closed `[lower, upper)` tick range, matching the schema's
/// `INT4RANGE` columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickRange {
    pub lower: i32,
    pub upper: i32,
}

impl TickRange {
    pub fn new(lower: i32, upper: i32) -> Result<Self> {
        if lower >= upper {
            bail!("TickRange lower {lower} must be < upper {upper}");
        }
        Ok(Self { lower, upper })
    }

    pub fn as_tuple(self) -> (i32, i32) {
        (self.lower, self.upper)
    }
}

impl From<RangeRecommendation> for TickRange {
    fn from(r: RangeRecommendation) -> Self {
        Self {
            lower: r.lower_tick,
            upper: r.upper_tick,
        }
    }
}

/// What the engine produced after execution.
#[derive(Debug, Clone)]
pub struct RebalanceOutcome {
    /// Journal row id assigned when the intent was persisted.
    pub journal_id: i64,
    /// Signature of the on-chain transaction(s) bundled together by the
    /// submitter. For now we assume a single "batch" signature; if the
    /// real submitter (task #15) splits close/open into two txs we will
    /// extend this to a vector.
    pub tx_sig: String,
    /// Timestamp to pass back into `SignalEngine::on_rebalance_executed`.
    pub executed_at_secs: u64,
}

// -----------------------------------------------------------------------------
// Ports (traits) — the real impls live elsewhere (#15 for submitter, #18
// for journal). The engine is written against these so it is unit-testable
// without a database or an RPC.
// -----------------------------------------------------------------------------

/// Journal port: persist the rebalance intent before touching chain, then
/// mark it completed once the submitter returns a signature.
#[async_trait]
pub trait RebalanceJournal: Send + Sync {
    /// Append a `Pending` row describing what is about to happen. Must
    /// return the row id so it can be completed later. The id is also
    /// what the engine returns in [`RebalanceOutcome::journal_id`].
    async fn record_intent(&self, intent: &RebalanceIntent<'_>) -> Result<i64>;

    /// Update the row created by [`Self::record_intent`] to reflect the
    /// successful transaction signature. Called only after the submitter
    /// returns `Ok`.
    async fn mark_completed(&self, journal_id: i64, tx_sig: &str) -> Result<()>;
}

/// Borrowed view of a rebalance intent used at persistence time.
///
/// Keeping this as a borrow (rather than another owned struct) avoids
/// making the journal care about `Pubkey`/`RangeRecommendation` types.
#[derive(Debug)]
pub struct RebalanceIntent<'a> {
    pub position_id: i64,
    pub old_range: (i32, i32),
    pub new_range: (i32, i32),
    /// Stable reason label, not `Debug`. See [`reason_label`].
    pub reason: &'a str,
}

/// Submitter port: build and send the close/collect/open sequence.
///
/// The real implementation (task #15) will construct Solana
/// `Instruction`s via the Whirlpool program client, sign them, and send
/// them. For now the engine just delegates the whole flow here so tests
/// can substitute an in-memory submitter that records the plan.
#[async_trait]
pub trait TxSubmitter: Send + Sync {
    /// Execute the full close → collect → open → increase-liquidity
    /// sequence described by `plan`, returning a bundle signature
    /// (placeholder shape until #15 lands and we learn whether close
    /// and open need separate txs).
    async fn submit_rebalance(&self, plan: &RebalancePlan) -> Result<String>;
}

// -----------------------------------------------------------------------------
// Engine
// -----------------------------------------------------------------------------

/// Executes rebalance plans against the chain via a [`TxSubmitter`] and
/// records the audit trail via a [`RebalanceJournal`].
///
/// Stored as `Arc<dyn _>` so callers can freely clone/share submitters
/// and journals across tasks without the engine leaking generic
/// parameters.
pub struct RebalanceEngine {
    submitter: std::sync::Arc<dyn TxSubmitter>,
    journal: std::sync::Arc<dyn RebalanceJournal>,
}

impl RebalanceEngine {
    pub fn new(
        submitter: std::sync::Arc<dyn TxSubmitter>,
        journal: std::sync::Arc<dyn RebalanceJournal>,
    ) -> Self {
        Self { submitter, journal }
    }

    /// Convenience constructor that takes a [`RebalanceSignal`] and a
    /// base [`RebalancePlan`] (without the signal-derived fields) and
    /// fills in `reason` / `target_range` from the signal. Returns
    /// `Ok(None)` for `Hold` so callers can `if let Some(outcome) = ...`.
    pub async fn execute_signal(
        &self,
        mut plan: RebalancePlan,
        signal: RebalanceSignal,
    ) -> Result<Option<RebalanceOutcome>> {
        match signal {
            RebalanceSignal::Hold => Ok(None),
            RebalanceSignal::Rebalance {
                reason,
                target_range,
            } => {
                plan.reason = reason;
                plan.target_range = target_range;
                let outcome = self.execute(plan).await?;
                Ok(Some(outcome))
            }
        }
    }

    /// Execute a fully-formed plan. Writes the intent to the journal,
    /// submits the on-chain sequence, and then marks the journal row
    /// completed. Errors from the submitter leave the journal row in
    /// the `Pending` state so the recovery path can see it.
    pub async fn execute(&self, plan: RebalancePlan) -> Result<RebalanceOutcome> {
        let span = rebalance_span(&plan.position_mint.to_string(), reason_label(plan.reason));
        async move {
            let intent = RebalanceIntent {
                position_id: plan.position_id,
                old_range: plan.current_range.as_tuple(),
                new_range: (plan.target_range.lower_tick, plan.target_range.upper_tick),
                reason: reason_label(plan.reason),
            };
            let journal_id = self
                .journal
                .record_intent(&intent)
                .await
                .context("record rebalance intent")?;

            let tx_sig = self
                .submitter
                .submit_rebalance(&plan)
                .await
                .context("submit rebalance tx")?;

            self.journal
                .mark_completed(journal_id, &tx_sig)
                .await
                .context("mark rebalance completed")?;

            Ok(RebalanceOutcome {
                journal_id,
                tx_sig,
                executed_at_secs: plan.started_at_secs,
            })
        }
        .instrument(span)
        .await
    }
}

/// Stable string label for a [`RebalanceReason`]. Used in both the span
/// field and the persisted `reason` column; dashboards key on these
/// strings, so keep them stable even across Rust/serde upgrades.
///
/// **Do not** derive this via `Debug`: a rename refactor would silently
/// break every dashboard query.
pub fn reason_label(reason: RebalanceReason) -> &'static str {
    match reason {
        RebalanceReason::OutOfRange => "out_of_range",
        RebalanceReason::PnlBelowThreshold => "pnl_below_threshold",
        RebalanceReason::FeesBelowFloor => "fees_below_floor",
        RebalanceReason::Manual => "manual",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // ---- In-memory test doubles ---------------------------------------

    type RecordedIntent = (i64, (i32, i32), (i32, i32), String);

    #[derive(Default)]
    struct RecordingJournal {
        next_id: AtomicI64,
        intents: Mutex<Vec<RecordedIntent>>,
        completed: Mutex<Vec<(i64, String)>>,
    }

    #[async_trait]
    impl RebalanceJournal for RecordingJournal {
        async fn record_intent(&self, intent: &RebalanceIntent<'_>) -> Result<i64> {
            let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
            self.intents.lock().await.push((
                intent.position_id,
                intent.old_range,
                intent.new_range,
                intent.reason.to_string(),
            ));
            Ok(id)
        }
        async fn mark_completed(&self, journal_id: i64, tx_sig: &str) -> Result<()> {
            self.completed
                .lock()
                .await
                .push((journal_id, tx_sig.to_string()));
            Ok(())
        }
    }

    #[derive(Default)]
    struct CountingSubmitter {
        calls: AtomicUsize,
        fail: AtomicBool,
    }

    #[async_trait]
    impl TxSubmitter for CountingSubmitter {
        async fn submit_rebalance(&self, _plan: &RebalancePlan) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                bail!("boom");
            }
            Ok("FakeSig1111".to_string())
        }
    }

    fn sample_plan() -> RebalancePlan {
        RebalancePlan {
            position_id: 42,
            position_mint: Pubkey::new_unique(),
            current_range: TickRange::new(-2000, 2000).unwrap(),
            target_range: RangeRecommendation {
                lower_tick: -1000,
                upper_tick: 1000,
                expected_capital_efficiency_ppm: 5_000_000,
            },
            reason: RebalanceReason::OutOfRange,
            started_at_secs: 1_700_000_000,
        }
    }

    // ---- Unit tests ----------------------------------------------------

    #[test]
    fn reason_label_covers_every_variant() {
        assert_eq!(reason_label(RebalanceReason::OutOfRange), "out_of_range");
        assert_eq!(
            reason_label(RebalanceReason::PnlBelowThreshold),
            "pnl_below_threshold"
        );
        assert_eq!(
            reason_label(RebalanceReason::FeesBelowFloor),
            "fees_below_floor"
        );
        assert_eq!(reason_label(RebalanceReason::Manual), "manual");
    }

    #[test]
    fn tick_range_rejects_degenerate() {
        assert!(TickRange::new(10, 10).is_err());
        assert!(TickRange::new(20, 10).is_err());
        assert!(TickRange::new(-5, 5).is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_records_intent_then_submits_then_completes() {
        let journal = Arc::new(RecordingJournal::default());
        let submitter = Arc::new(CountingSubmitter::default());
        let engine = RebalanceEngine::new(
            Arc::clone(&submitter) as Arc<dyn TxSubmitter>,
            Arc::clone(&journal) as Arc<dyn RebalanceJournal>,
        );

        let outcome = engine.execute(sample_plan()).await.unwrap();
        assert_eq!(outcome.tx_sig, "FakeSig1111");
        assert_eq!(outcome.executed_at_secs, 1_700_000_000);

        let intents = journal.intents.lock().await;
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].0, 42);
        assert_eq!(intents[0].1, (-2000, 2000));
        assert_eq!(intents[0].2, (-1000, 1000));
        assert_eq!(intents[0].3, "out_of_range");

        let completed = journal.completed.lock().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, outcome.journal_id);
        assert_eq!(submitter.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn failed_submit_leaves_intent_pending() {
        let journal = Arc::new(RecordingJournal::default());
        let submitter = Arc::new(CountingSubmitter::default());
        submitter.fail.store(true, Ordering::SeqCst);
        let engine = RebalanceEngine::new(
            Arc::clone(&submitter) as Arc<dyn TxSubmitter>,
            Arc::clone(&journal) as Arc<dyn RebalanceJournal>,
        );

        let err = engine.execute(sample_plan()).await.unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("submit rebalance tx"), "got: {chain}");

        // Intent recorded but never completed — recovery path sees a
        // Pending row.
        assert_eq!(journal.intents.lock().await.len(), 1);
        assert!(journal.completed.lock().await.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_signal_holds_is_noop() {
        let journal = Arc::new(RecordingJournal::default());
        let submitter = Arc::new(CountingSubmitter::default());
        let engine = RebalanceEngine::new(
            Arc::clone(&submitter) as Arc<dyn TxSubmitter>,
            Arc::clone(&journal) as Arc<dyn RebalanceJournal>,
        );

        let outcome = engine
            .execute_signal(sample_plan(), RebalanceSignal::Hold)
            .await
            .unwrap();
        assert!(outcome.is_none());
        assert_eq!(submitter.calls.load(Ordering::SeqCst), 0);
        assert!(journal.intents.lock().await.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_signal_overrides_plan_reason_and_range() {
        let journal = Arc::new(RecordingJournal::default());
        let submitter = Arc::new(CountingSubmitter::default());
        let engine = RebalanceEngine::new(
            Arc::clone(&submitter) as Arc<dyn TxSubmitter>,
            Arc::clone(&journal) as Arc<dyn RebalanceJournal>,
        );

        let new_target = RangeRecommendation {
            lower_tick: -500,
            upper_tick: 500,
            expected_capital_efficiency_ppm: 9_000_000,
        };
        let outcome = engine
            .execute_signal(
                sample_plan(),
                RebalanceSignal::Rebalance {
                    reason: RebalanceReason::PnlBelowThreshold,
                    target_range: new_target,
                },
            )
            .await
            .unwrap()
            .expect("rebalance should have executed");

        assert_eq!(outcome.tx_sig, "FakeSig1111");
        let intents = journal.intents.lock().await;
        // The plan's reason/target were overridden by the signal.
        assert_eq!(intents[0].3, "pnl_below_threshold");
        assert_eq!(intents[0].2, (-500, 500));
    }
}
