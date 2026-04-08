//! Rebalance signal generator.
//!
//! A pure state machine that consumes a stream of [`MarketTick`]s and
//! emits a [`RebalanceSignal`] on each tick. The engine is deliberately
//! I/O-free: the data layer feeds it ticks, and the execution layer
//! reacts to the signal. This makes it trivial to unit-test with
//! deterministic event streams and to reuse inside the backtest engine
//! (#20).
//!
//! ## Triggers
//!
//! Per task #13 the engine fires a `Rebalance` on any of:
//!
//! - **`OutOfRange`** — the current price has been continuously outside
//!   `[lower_price, upper_price]` for at least `min_out_of_range`. The
//!   continuous-duration constraint avoids flapping on a single bad print.
//! - **`PnlBelowThreshold`** — the latest [`PnlSnapshot`] reports
//!   `net < -pnl_loss_threshold_quote`. The threshold is configured in
//!   absolute quote units, not a percentage, because position sizes vary.
//! - **`FeesBelowFloor`** — extrapolated fees-per-day across the active
//!   window have dropped below `min_fees_per_day_quote`. Only fires after
//!   the position has been live for at least `fee_window_min` so a fresh
//!   position isn't penalised for having no history.
//! - **`Manual`** — caller-provided override (e.g. CLI / API request).
//!
//! Triggers are checked in the order above; the first one to fire wins
//! and is reported via [`RebalanceReason`]. This means a position that is
//! both out of range and bleeding P&L will report `OutOfRange` — the
//! cheaper-to-act, less-judgement-call reason — which is what the
//! execution layer wants to log.
//!
//! ## What this engine does NOT do
//!
//! - It does not call the range optimizer; the new range is supplied by
//!   the caller via [`SignalEngine::set_target_range`] (typically the
//!   output of `range::RangeStrategy::recommend` from the previous tick).
//!   This keeps the engine pure and the caller in control of strategy
//!   composition.
//! - It does not collect fees, close positions, or send transactions.
//! - It does not compute IL or P&L; both arrive pre-computed in the tick.

use std::time::Duration;

use anyhow::{anyhow, bail, Result};

use crate::strategy::pnl::PnlSnapshot;
use crate::strategy::range::RangeRecommendation;

/// One observation of pool/position state, fed to [`SignalEngine::on_tick`].
#[derive(Debug, Clone, Copy)]
pub struct MarketTick {
    /// Monotonically non-decreasing wall-clock timestamp (seconds since
    /// some fixed epoch — the engine only ever subtracts ticks, so the
    /// epoch is arbitrary as long as it is consistent across calls).
    pub timestamp_secs: u64,
    /// Current pool price (display units, quote per base).
    pub current_price: f64,
    /// Lower bound of the active LP range (display units).
    pub lower_price: f64,
    /// Upper bound of the active LP range (display units).
    pub upper_price: f64,
    /// Most recent P&L snapshot for the active position.
    pub pnl: PnlSnapshot,
    /// Fees earned **since `window_started_at`** (i.e. window-scoped,
    /// not lifetime), in quote units. Used together with the active-
    /// window duration to compute fees-per-day. The caller must reset
    /// this accumulator when feeding the first tick after a rebalance —
    /// the engine tracks the window start time but not the fees.
    pub fees_earned_quote: f64,
    /// If `true`, the caller is requesting a manual rebalance. Lowest
    /// priority of the four triggers.
    pub manual_request: bool,
}

/// What the engine recommends doing on each tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebalanceSignal {
    Hold,
    Rebalance {
        reason: RebalanceReason,
        target_range: RangeRecommendation,
    },
}

/// Why the engine fired a rebalance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebalanceReason {
    OutOfRange,
    PnlBelowThreshold,
    FeesBelowFloor,
    Manual,
}

/// Configurable thresholds. Each threshold can be disabled individually
/// by setting it to its sentinel value (documented inline).
#[derive(Debug, Clone, Copy)]
pub struct SignalConfig {
    /// Minimum continuous duration the price must spend outside the range
    /// before `OutOfRange` fires. Set to `Duration::MAX` to disable.
    pub min_out_of_range: Duration,
    /// `pnl.net` must drop below `-pnl_loss_threshold_quote` to fire
    /// `PnlBelowThreshold`. Set to `f64::INFINITY` to disable.
    pub pnl_loss_threshold_quote: f64,
    /// Minimum fees-per-day (quote units) below which `FeesBelowFloor`
    /// fires. Set to `0.0` to disable.
    pub min_fees_per_day_quote: f64,
    /// Minimum active-window duration before `FeesBelowFloor` is allowed
    /// to fire. Prevents penalising a fresh position with no history.
    pub fee_window_min: Duration,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            min_out_of_range: Duration::from_secs(300), // 5 min
            pnl_loss_threshold_quote: f64::INFINITY,    // disabled by default
            min_fees_per_day_quote: 0.0,                // disabled by default
            fee_window_min: Duration::from_secs(3600),  // 1 h
        }
    }
}

/// State machine that converts ticks into rebalance signals.
#[derive(Debug, Clone)]
pub struct SignalEngine {
    config: SignalConfig,
    target_range: Option<RangeRecommendation>,
    /// First timestamp at which the price was observed *outside* the
    /// active range, reset to `None` on every in-range tick.
    out_of_range_since: Option<u64>,
    /// Timestamp at which the active position window opened (the first
    /// tick the engine ever saw, or the most recent rebalance).
    window_started_at: Option<u64>,
    /// Last observed tick timestamp, used as a monotonicity guard.
    last_tick_ts: Option<u64>,
}

impl SignalEngine {
    pub fn new(config: SignalConfig) -> Self {
        Self {
            config,
            target_range: None,
            out_of_range_since: None,
            window_started_at: None,
            last_tick_ts: None,
        }
    }

    /// Update the target range that will be used as the rebalance payload.
    /// Typically called once per tick by the caller, fed by
    /// `range::RangeStrategy::recommend`.
    pub fn set_target_range(&mut self, target: RangeRecommendation) {
        self.target_range = Some(target);
    }

    /// Notify the engine that a rebalance has just been executed: resets
    /// the out-of-range timer and the fee window so the next tick starts
    /// fresh.
    pub fn on_rebalance_executed(&mut self, at_ts: u64) {
        self.out_of_range_since = None;
        self.window_started_at = Some(at_ts);
    }

    /// Process one tick. Returns the recommended action.
    ///
    /// # Errors
    /// - Non-finite or non-positive prices in the tick.
    /// - `lower_price >= upper_price`.
    /// - Tick timestamp goes backwards relative to the previous tick.
    /// - A rebalance trigger fires but no `target_range` has been set.
    pub fn on_tick(&mut self, tick: MarketTick) -> Result<RebalanceSignal> {
        validate_tick(&tick)?;
        if let Some(prev) = self.last_tick_ts {
            if tick.timestamp_secs < prev {
                bail!(
                    "tick timestamp {} is before previous {}",
                    tick.timestamp_secs,
                    prev
                );
            }
        }
        self.last_tick_ts = Some(tick.timestamp_secs);
        if self.window_started_at.is_none() {
            self.window_started_at = Some(tick.timestamp_secs);
        }

        // Out-of-range tracking.
        let in_range =
            tick.current_price >= tick.lower_price && tick.current_price <= tick.upper_price;
        if in_range {
            self.out_of_range_since = None;
        } else if self.out_of_range_since.is_none() {
            self.out_of_range_since = Some(tick.timestamp_secs);
        }

        match self.classify(&tick) {
            None => Ok(RebalanceSignal::Hold),
            Some(reason) => {
                let target_range = self.target_range.ok_or_else(|| {
                    anyhow!("rebalance triggered ({:?}) but no target_range set", reason)
                })?;
                Ok(RebalanceSignal::Rebalance {
                    reason,
                    target_range,
                })
            }
        }
    }

    /// First trigger that fires for `tick`, in priority order. Returns
    /// `None` for hold.
    fn classify(&self, tick: &MarketTick) -> Option<RebalanceReason> {
        // 1. Out of range for >= min_out_of_range.
        if let Some(since) = self.out_of_range_since {
            let dur = Duration::from_secs(tick.timestamp_secs.saturating_sub(since));
            if dur >= self.config.min_out_of_range {
                return Some(RebalanceReason::OutOfRange);
            }
        }
        // 2. P&L below threshold.
        if tick.pnl.net < -self.config.pnl_loss_threshold_quote {
            return Some(RebalanceReason::PnlBelowThreshold);
        }
        // 3. Fees-per-day below floor (only after fee_window_min has elapsed).
        if self.config.min_fees_per_day_quote > 0.0 {
            if let Some(start) = self.window_started_at {
                let elapsed = Duration::from_secs(tick.timestamp_secs.saturating_sub(start));
                if elapsed >= self.config.fee_window_min && elapsed.as_secs() > 0 {
                    let days = elapsed.as_secs_f64() / 86_400.0;
                    let fees_per_day = tick.fees_earned_quote / days;
                    if fees_per_day < self.config.min_fees_per_day_quote {
                        return Some(RebalanceReason::FeesBelowFloor);
                    }
                }
            }
        }
        // 4. Manual override is the lowest priority.
        if tick.manual_request {
            return Some(RebalanceReason::Manual);
        }
        None
    }
}

fn validate_tick(t: &MarketTick) -> Result<()> {
    for (name, v) in [
        ("current_price", t.current_price),
        ("lower_price", t.lower_price),
        ("upper_price", t.upper_price),
    ] {
        if !v.is_finite() || v <= 0.0 {
            bail!("{} must be finite and positive, got {}", name, v);
        }
    }
    if t.lower_price >= t.upper_price {
        bail!("lower_price must be < upper_price");
    }
    if !t.fees_earned_quote.is_finite() || t.fees_earned_quote < 0.0 {
        bail!(
            "fees_earned_quote must be finite and non-negative, got {}",
            t.fees_earned_quote
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> RangeRecommendation {
        RangeRecommendation {
            lower_tick: -1000,
            upper_tick: 1000,
            expected_capital_efficiency_ppm: 5_000_000,
        }
    }

    fn pnl(net: f64) -> PnlSnapshot {
        PnlSnapshot {
            fees_earned: 0.0,
            il_quote: net,
            net,
        }
    }

    fn tick(ts: u64, price: f64, lower: f64, upper: f64, net: f64) -> MarketTick {
        MarketTick {
            timestamp_secs: ts,
            current_price: price,
            lower_price: lower,
            upper_price: upper,
            pnl: pnl(net),
            fees_earned_quote: 0.0,
            manual_request: false,
        }
    }

    #[test]
    fn holds_when_in_range_and_thresholds_disabled() {
        let mut eng = SignalEngine::new(SignalConfig::default());
        eng.set_target_range(target());
        let s = eng.on_tick(tick(0, 100.0, 80.0, 120.0, 0.0)).unwrap();
        assert_eq!(s, RebalanceSignal::Hold);
    }

    #[test]
    fn out_of_range_fires_only_after_min_duration() {
        let cfg = SignalConfig {
            min_out_of_range: Duration::from_secs(60),
            ..SignalConfig::default()
        };
        let mut eng = SignalEngine::new(cfg);
        eng.set_target_range(target());

        // First tick: out of range, but no time has elapsed yet.
        assert_eq!(
            eng.on_tick(tick(0, 130.0, 80.0, 120.0, 0.0)).unwrap(),
            RebalanceSignal::Hold
        );
        // 30 s later, still out of range — not yet 60 s.
        assert_eq!(
            eng.on_tick(tick(30, 130.0, 80.0, 120.0, 0.0)).unwrap(),
            RebalanceSignal::Hold
        );
        // 60 s later — fires.
        match eng.on_tick(tick(60, 130.0, 80.0, 120.0, 0.0)).unwrap() {
            RebalanceSignal::Rebalance { reason, .. } => {
                assert_eq!(reason, RebalanceReason::OutOfRange)
            }
            _ => panic!("expected rebalance"),
        }
    }

    #[test]
    fn out_of_range_timer_resets_on_in_range_tick() {
        let cfg = SignalConfig {
            min_out_of_range: Duration::from_secs(60),
            ..SignalConfig::default()
        };
        let mut eng = SignalEngine::new(cfg);
        eng.set_target_range(target());

        eng.on_tick(tick(0, 130.0, 80.0, 120.0, 0.0)).unwrap();
        eng.on_tick(tick(30, 100.0, 80.0, 120.0, 0.0)).unwrap(); // back in range
        eng.on_tick(tick(60, 130.0, 80.0, 120.0, 0.0)).unwrap(); // out again, fresh timer
        let s = eng.on_tick(tick(80, 130.0, 80.0, 120.0, 0.0)).unwrap();
        assert_eq!(s, RebalanceSignal::Hold); // only 20 s of new out-of-range
    }

    #[test]
    fn pnl_below_threshold_fires() {
        let cfg = SignalConfig {
            pnl_loss_threshold_quote: 100.0,
            ..SignalConfig::default()
        };
        let mut eng = SignalEngine::new(cfg);
        eng.set_target_range(target());

        match eng.on_tick(tick(0, 100.0, 80.0, 120.0, -150.0)).unwrap() {
            RebalanceSignal::Rebalance { reason, .. } => {
                assert_eq!(reason, RebalanceReason::PnlBelowThreshold)
            }
            _ => panic!("expected rebalance"),
        }
    }

    #[test]
    fn fees_below_floor_after_window() {
        let cfg = SignalConfig {
            min_fees_per_day_quote: 100.0,
            fee_window_min: Duration::from_secs(3600),
            ..SignalConfig::default()
        };
        let mut eng = SignalEngine::new(cfg);
        eng.set_target_range(target());

        // Window opens at t=0.
        let mut t = tick(0, 100.0, 80.0, 120.0, 0.0);
        t.fees_earned_quote = 0.0;
        assert_eq!(eng.on_tick(t).unwrap(), RebalanceSignal::Hold);

        // 30 min in: below window minimum, hold.
        let mut t = tick(1800, 100.0, 80.0, 120.0, 0.0);
        t.fees_earned_quote = 0.5;
        assert_eq!(eng.on_tick(t).unwrap(), RebalanceSignal::Hold);

        // 2 h in: fees-per-day = 0.5 * 12 = 6, below 100/day floor → fires.
        let mut t = tick(7200, 100.0, 80.0, 120.0, 0.0);
        t.fees_earned_quote = 0.5;
        match eng.on_tick(t).unwrap() {
            RebalanceSignal::Rebalance { reason, .. } => {
                assert_eq!(reason, RebalanceReason::FeesBelowFloor)
            }
            other => panic!("expected rebalance, got {:?}", other),
        }
    }

    #[test]
    fn manual_request_is_lowest_priority() {
        let cfg = SignalConfig {
            min_out_of_range: Duration::from_secs(0),
            ..SignalConfig::default()
        };
        let mut eng = SignalEngine::new(cfg);
        eng.set_target_range(target());

        // Out of range AND manual: out-of-range wins.
        let mut t = tick(0, 130.0, 80.0, 120.0, 0.0);
        t.manual_request = true;
        match eng.on_tick(t).unwrap() {
            RebalanceSignal::Rebalance { reason, .. } => {
                assert_eq!(reason, RebalanceReason::OutOfRange)
            }
            _ => panic!("expected rebalance"),
        }

        // In range, only manual → manual fires.
        let mut eng = SignalEngine::new(SignalConfig::default());
        eng.set_target_range(target());
        let mut t = tick(0, 100.0, 80.0, 120.0, 0.0);
        t.manual_request = true;
        match eng.on_tick(t).unwrap() {
            RebalanceSignal::Rebalance { reason, .. } => {
                assert_eq!(reason, RebalanceReason::Manual)
            }
            _ => panic!("expected rebalance"),
        }
    }

    #[test]
    fn rebalance_without_target_range_errors() {
        let cfg = SignalConfig {
            min_out_of_range: Duration::from_secs(0),
            ..SignalConfig::default()
        };
        let mut eng = SignalEngine::new(cfg);
        // No set_target_range call.
        assert!(eng.on_tick(tick(0, 130.0, 80.0, 120.0, 0.0)).is_err());
    }

    #[test]
    fn on_rebalance_executed_resets_state() {
        let cfg = SignalConfig {
            min_out_of_range: Duration::from_secs(60),
            ..SignalConfig::default()
        };
        let mut eng = SignalEngine::new(cfg);
        eng.set_target_range(target());

        eng.on_tick(tick(0, 130.0, 80.0, 120.0, 0.0)).unwrap();
        eng.on_tick(tick(60, 130.0, 80.0, 120.0, 0.0)).unwrap(); // would-fire tick
        eng.on_rebalance_executed(60);
        // Even though price is still out, the timer has been reset.
        let s = eng.on_tick(tick(70, 130.0, 80.0, 120.0, 0.0)).unwrap();
        assert_eq!(s, RebalanceSignal::Hold);
    }

    #[test]
    fn rejects_invalid_inputs() {
        let mut eng = SignalEngine::new(SignalConfig::default());
        eng.set_target_range(target());
        assert!(eng.on_tick(tick(0, 0.0, 80.0, 120.0, 0.0)).is_err());
        assert!(eng.on_tick(tick(0, 100.0, 120.0, 80.0, 0.0)).is_err());

        // Backwards timestamp.
        eng.on_tick(tick(100, 100.0, 80.0, 120.0, 0.0)).unwrap();
        assert!(eng.on_tick(tick(50, 100.0, 80.0, 120.0, 0.0)).is_err());
    }
}
