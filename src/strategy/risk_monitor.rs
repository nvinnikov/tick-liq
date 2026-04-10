// Plans 02 and 03 wire these types into the DB layer and watch loop.
// Until then suppress dead-code lints for this entire module.
#![allow(dead_code)]

use chrono::{DateTime, Utc};

use crate::storage::writer::PnlSnapshot;

/// Persisted state for a single pool's risk monitor.
/// Stored in DB (plan 02) and loaded at watch startup.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used by Plan 02 (DB persistence) and Plan 03 (watch-loop wiring)
pub struct RiskState {
    pub pool_address: String,
    pub peak_pnl: f64,
    pub current_drawdown_pct: f64,
    pub pause_flag: bool,
    pub halt_flag: bool,
    pub updated_at: DateTime<Utc>,
}

/// Actions returned by [`RiskMonitor::evaluate`].
///
/// `PartialEq` is derived so tests can use `assert_eq!` on variants.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // Variants consumed by Plan 03 (watch-loop wiring)
pub enum RiskAction {
    /// No limit breached — proceed normally.
    Continue,
    /// IL exceeded threshold; rebalancing should be paused.
    PauseRebalancing { il_pct: f64 },
    /// IL dropped back below threshold; rebalancing can resume.
    ResumeRebalancing { il_pct: f64 },
    /// Drawdown exceeded threshold; halt all activity.
    HaltAll { drawdown_pct: f64 },
    /// Drift margin ratio below threshold; close hedge only.
    CloseDriftHedge { margin_ratio: f64 },
}

/// Pure-state risk evaluator.
///
/// `evaluate()` is synchronous and infallible — it takes a `PnlSnapshot` plus
/// an externally-fetched `drift_margin_ratio` so the method remains testable
/// without RPC mocking.
#[allow(dead_code)] // Constructed by Plan 03 (watch-loop wiring)
pub struct RiskMonitor {
    pub state: RiskState,
    max_drawdown_pct: Option<f64>,
    max_il_pct: Option<f64>,
    drift_min_margin_ratio: Option<f64>,
}

impl RiskMonitor {
    /// Create a new `RiskMonitor`.
    ///
    /// * `state` — initial persisted risk state (loaded from DB or freshly seeded)
    /// * `max_drawdown_pct` — drawdown threshold as a percentage (e.g. `15.0` = 15 %).
    ///   `None` disables drawdown checking.
    /// * `max_il_pct` — IL threshold as a percentage of `position_value`.
    ///   `None` disables IL checking.
    /// * `drift_min_margin_ratio` — minimum acceptable Drift margin ratio (0.0–1.0).
    ///   `None` disables Drift margin checking.
    #[allow(dead_code)]
    pub fn new(
        state: RiskState,
        max_drawdown_pct: Option<f64>,
        max_il_pct: Option<f64>,
        drift_min_margin_ratio: Option<f64>,
    ) -> Self {
        Self {
            state,
            max_drawdown_pct,
            max_il_pct,
            drift_min_margin_ratio,
        }
    }

    /// Evaluate all risk limits for the given P&L snapshot.
    ///
    /// Evaluation order (per D-05, D-06):
    /// 1. halt_flag gate
    /// 2. Peak P&L high-water mark update
    /// 3. Drawdown check
    /// 4. IL check
    /// 5. Drift margin check
    /// 6. Continue
    #[allow(dead_code)]
    pub fn evaluate(&mut self, snap: &PnlSnapshot, drift_margin_ratio: Option<f64>) -> RiskAction {
        // --- 1. halt_flag gate ---
        if self.state.halt_flag {
            self.state.updated_at = Utc::now();
            return RiskAction::HaltAll {
                drawdown_pct: self.state.current_drawdown_pct,
            };
        }

        // --- 2. Update peak P&L high-water mark ---
        if snap.net_pnl > self.state.peak_pnl {
            self.state.peak_pnl = snap.net_pnl;
        }

        // --- 3. Drawdown check (skip when no peak established) ---
        if let Some(max_dd) = self.max_drawdown_pct {
            if self.state.peak_pnl > 0.0 {
                let drawdown_pct =
                    (self.state.peak_pnl - snap.net_pnl) / self.state.peak_pnl * 100.0;
                self.state.current_drawdown_pct = drawdown_pct;

                if drawdown_pct > max_dd {
                    self.state.halt_flag = true;
                    self.state.updated_at = Utc::now();
                    return RiskAction::HaltAll { drawdown_pct };
                }
            }
        }

        // --- 4. IL check ---
        let il_pct = if snap.position_value > 0.0 {
            snap.il_usd.abs() / snap.position_value * 100.0
        } else {
            0.0
        };

        if let Some(max_il) = self.max_il_pct {
            if il_pct > max_il {
                if !self.state.pause_flag {
                    self.state.pause_flag = true;
                    self.state.updated_at = Utc::now();
                    return RiskAction::PauseRebalancing { il_pct };
                } else {
                    // Already paused — propagate
                    self.state.updated_at = Utc::now();
                    return RiskAction::PauseRebalancing { il_pct };
                }
            } else if self.state.pause_flag {
                // IL dropped back below threshold — auto-resume
                self.state.pause_flag = false;
                self.state.updated_at = Utc::now();
                return RiskAction::ResumeRebalancing { il_pct };
            }
        }

        // --- 5. Drift margin check ---
        if let Some(min_ratio) = self.drift_min_margin_ratio {
            if let Some(ratio) = drift_margin_ratio {
                if ratio < min_ratio {
                    self.state.updated_at = Utc::now();
                    return RiskAction::CloseDriftHedge {
                        margin_ratio: ratio,
                    };
                }
            }
        }

        // --- 6. Continue ---
        self.state.updated_at = Utc::now();
        RiskAction::Continue
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_state(
        pool_address: &str,
        peak_pnl: f64,
        pause_flag: bool,
        halt_flag: bool,
    ) -> RiskState {
        RiskState {
            pool_address: pool_address.to_string(),
            peak_pnl,
            current_drawdown_pct: 0.0,
            pause_flag,
            halt_flag,
            updated_at: Utc::now(),
        }
    }

    fn make_snap(net_pnl: f64, il_usd: f64, position_value: f64) -> PnlSnapshot {
        PnlSnapshot {
            mint: "MINT".to_string(),
            pool_address: "POOL".to_string(),
            fees_earned: 0.0,
            il_usd,
            net_pnl,
            position_value,
            price: 100.0,
            observed_at: Utc::now(),
        }
    }

    fn monitor_all(
        state: RiskState,
        max_dd: Option<f64>,
        max_il: Option<f64>,
        drift_min: Option<f64>,
    ) -> RiskMonitor {
        RiskMonitor::new(state, max_dd, max_il, drift_min)
    }

    // -----------------------------------------------------------------------
    // halt_flag gate
    // -----------------------------------------------------------------------

    #[test]
    fn halt_flag_returns_halt_all_immediately() {
        let state = make_state("POOL", 100.0, false, true);
        let mut rm = monitor_all(state, Some(15.0), Some(4.0), None);
        let snap = make_snap(90.0, -5.0, 1000.0);
        // halt_flag is true — no further checks should run
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::HaltAll { drawdown_pct: 0.0 });
    }

    // -----------------------------------------------------------------------
    // Drawdown checks
    // -----------------------------------------------------------------------

    #[test]
    fn drawdown_breach_returns_halt_all() {
        let state = make_state("POOL", 100.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), None, None);
        // peak=100, net_pnl=80 -> drawdown=20% > 15% threshold
        let snap = make_snap(80.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::HaltAll { drawdown_pct: 20.0 });
        assert!(
            rm.state.halt_flag,
            "halt_flag must be set after drawdown breach"
        );
    }

    #[test]
    fn drawdown_skipped_when_peak_not_positive() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), None, None);
        // peak_pnl <= 0 — drawdown check must be skipped
        let snap = make_snap(-50.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
    }

    #[test]
    fn drawdown_not_triggered_below_threshold() {
        let state = make_state("POOL", 100.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), None, None);
        // drawdown=10% < 15% threshold
        let snap = make_snap(90.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
        assert!(!rm.state.halt_flag);
    }

    // -----------------------------------------------------------------------
    // High-water mark
    // -----------------------------------------------------------------------

    #[test]
    fn peak_pnl_only_increases() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, None);
        // First tick: net_pnl=200 -> peak should be 200
        rm.evaluate(&make_snap(200.0, 0.0, 1000.0), None);
        assert_eq!(rm.state.peak_pnl, 200.0);
        // Second tick: net_pnl=150 -> peak must remain 200
        rm.evaluate(&make_snap(150.0, 0.0, 1000.0), None);
        assert_eq!(rm.state.peak_pnl, 200.0);
    }

    // -----------------------------------------------------------------------
    // IL checks
    // -----------------------------------------------------------------------

    #[test]
    fn il_breach_returns_pause_rebalancing() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // IL = |-50| / 1000 * 100 = 5% > 4%
        let snap = make_snap(0.0, -50.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::PauseRebalancing { il_pct: 5.0 });
        assert!(rm.state.pause_flag);
    }

    #[test]
    fn il_recovery_returns_resume_rebalancing() {
        let state = make_state("POOL", 0.0, true, false); // pause_flag=true
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // IL = |-30| / 1000 * 100 = 3% <= 4%
        let snap = make_snap(0.0, -30.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::ResumeRebalancing { il_pct: 3.0 });
        assert!(!rm.state.pause_flag);
    }

    #[test]
    fn il_still_above_threshold_while_paused_returns_pause_rebalancing() {
        let state = make_state("POOL", 0.0, true, false); // pause_flag=true
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // IL = |-50| / 1000 * 100 = 5% > 4%
        let snap = make_snap(0.0, -50.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::PauseRebalancing { il_pct: 5.0 });
        assert!(rm.state.pause_flag, "pause_flag must remain set");
    }

    #[test]
    fn il_no_hysteresis_threshold_same_for_pause_and_resume() {
        // Pause threshold == resume threshold (D-09)
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // Exactly at threshold: 4% == 4% -> NOT a breach (> check)
        let snap = make_snap(0.0, -40.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
    }

    #[test]
    fn il_position_value_zero_yields_zero_il_pct() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // position_value=0 -> il_pct must be 0, not NaN/inf
        let snap = make_snap(0.0, -50.0, 0.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
    }

    // -----------------------------------------------------------------------
    // Evaluation order: drawdown fires before IL
    // -----------------------------------------------------------------------

    #[test]
    fn drawdown_fires_before_il_check() {
        // Both drawdown and IL are breached; drawdown must win
        let state = make_state("POOL", 100.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), Some(4.0), None);
        // drawdown = 20% > 15%; IL = 5% > 4%
        let snap = make_snap(80.0, -50.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert!(
            matches!(action, RiskAction::HaltAll { .. }),
            "expected HaltAll, got {action:?}"
        );
    }

    // -----------------------------------------------------------------------
    // All limits disabled
    // -----------------------------------------------------------------------

    #[test]
    fn all_limits_none_returns_continue() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, None);
        let snap = make_snap(-1000.0, -500.0, 100.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
    }

    // -----------------------------------------------------------------------
    // Drift margin checks
    // -----------------------------------------------------------------------

    #[test]
    fn drift_margin_below_threshold_returns_close_drift_hedge() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, Some(0.10));
        let snap = make_snap(0.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, Some(0.05));
        assert_eq!(action, RiskAction::CloseDriftHedge { margin_ratio: 0.05 });
    }

    #[test]
    fn drift_margin_above_threshold_returns_continue() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, Some(0.10));
        let snap = make_snap(0.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, Some(0.15));
        assert_eq!(action, RiskAction::Continue);
    }

    #[test]
    fn drift_min_margin_none_returns_continue() {
        // drift_min_margin_ratio = None -> Drift check disabled
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, None);
        let snap = make_snap(0.0, 0.0, 1000.0);
        // Even if ratio is very low, disabled check must not fire
        let action = rm.evaluate(&snap, Some(0.001));
        assert_eq!(action, RiskAction::Continue);
    }
}
