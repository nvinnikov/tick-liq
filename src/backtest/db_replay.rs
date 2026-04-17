//! DB-mode backtest: replays real `pool_ticks` rows chronologically,
//! computing fees from `fee_growth_global_a` deltas and IL via CLMM math.
//!
//! Produces the same `BacktestResult` schema as the GBM `backtest::run()`
//! so both modes share a single display / reporting path.
//!
//! # Fee approximation (documented)
//! Exact fee_growth_inside requires per-tick-array data not stored in
//! `pool_ticks`. This implementation uses a liquidity-share approximation:
//!
//!   fees_step = (fee_growth_delta_a * position_liquidity / pool_liquidity)
//!               / 2^64   (de-scaling X64 fixed-point)
//!
//! This is an intentional, documented approximation (T-03-09 accepted risk).

use anyhow::{bail, Result};

use crate::backtest::{price_to_tick, BacktestResult, DayResult, ParamsSnapshot};
use crate::math::il::compute_il;
use crate::math::sqrt_price::sqrt_q64_to_price;
use crate::storage::tick_reader::PoolTickRow;
use crate::strategy::{self, RebalanceConfig, RebalanceDecision};

/// Parameters specific to the DB-mode replay (complements the shared fields
/// that are also in `BacktestParams`).
pub struct DbBacktestInput {
    /// Position value in USD at open (used for IL/APY accounting).
    pub initial_value_usd: f64,
    /// Price when the position was originally opened.
    pub entry_price: f64,
    /// Lower bound of the LP range in price units.
    pub price_lower: f64,
    /// Upper bound of the LP range in price units.
    pub price_upper: f64,
    /// Pool fee rate in basis points (e.g. 5.0 = 0.05%).
    pub fee_rate_bps: f64,
    /// Tick spacing for the pool (used by price_to_tick alignment).
    pub tick_spacing: i32,
    /// Liquidity units held by this position (operator-supplied at CLI layer).
    pub position_liquidity: u128,
    /// Rebalance configuration; `rebalance_out_of_range = false` disables it.
    pub rebalance_cfg: RebalanceConfig,
    /// Width factor applied to the lower bound on rebalance
    /// (e.g. 0.95 → new_lower = price * 0.95).
    pub range_factor_lower: f64,
    /// Width factor applied to the upper bound on rebalance
    /// (e.g. 1.05 → new_upper = price * 1.05).
    pub range_factor_upper: f64,
}

/// Compute fee-growth delta with u128 wrapping support.
///
/// Handles the case where `fee_growth_global` has wrapped around u128::MAX
/// by using `wrapping_sub` (T-03-05 mitigation).
#[inline]
fn fee_growth_delta(prev: u128, curr: u128) -> u128 {
    curr.wrapping_sub(prev)
}

/// Replay a chronological slice of `PoolTickRow` and produce a `BacktestResult`
/// with the same schema as the GBM `backtest::run()`.
///
/// # Errors
/// Returns `Err` if `ticks` is empty (nothing to replay).
///
/// # Fee approximation
/// Uses liquidity-share approximation (documented above). Exact
/// `fee_growth_inside` computation is deferred to v2 (T-03-09 accepted).
pub fn run_db_backtest(input: DbBacktestInput, ticks: &[PoolTickRow]) -> Result<BacktestResult> {
    if ticks.is_empty() {
        bail!("run_db_backtest: empty tick stream — nothing to replay");
    }

    // Derive integer tick bounds from the input price range.
    let mut tick_lower = price_to_tick(input.price_lower, input.tick_spacing);
    let mut tick_upper = price_to_tick(input.price_upper, input.tick_spacing);

    // Track current range boundaries in price terms (updated on rebalance).
    let mut cur_entry_price = input.entry_price;
    let mut cur_price_lower = input.price_lower;
    let mut cur_price_upper = input.price_upper;

    let mut cumulative_fees: f64 = 0.0;
    let mut total_rebalances: u32 = 0;
    let mut days_in_range: u32 = 0;

    // Carry the previous fee_growth_global_a for delta computation.
    let mut prev_fg_a: u128 = ticks[0].fee_growth_global_a;

    // Roll-up accumulators for the current UTC calendar day.
    let mut current_day_num: u32 = 0; // 1-based day counter across the tick stream
    let mut current_day_date = ticks[0].time.date_naive();
    let mut day_was_in_range = false;

    let mut day_results: Vec<DayResult> = Vec::new();

    for (i, t) in ticks.iter().enumerate() {
        let price = sqrt_q64_to_price(t.sqrt_price);
        let day_date = t.time.date_naive();
        let in_range = t.tick_current >= tick_lower && t.tick_current <= tick_upper;

        // ── Day boundary ────────────────────────────────────────────────────
        if day_date != current_day_date {
            // Flush the completed day. `day_results` records `cumulative_fees`
            // (total since start of replay); per-day fee delta was dropped as
            // dead computation (IN-01). Restore if a future DayResult exposes
            // daily deltas explicitly.
            let il_frac = compute_il(cur_entry_price, price, cur_price_lower, cur_price_upper);
            let il_usd = il_frac * input.initial_value_usd;
            let net_pnl_usd = cumulative_fees + il_usd;

            if day_was_in_range {
                days_in_range += 1;
            }

            current_day_num += 1;
            day_results.push(DayResult {
                day: current_day_num,
                price,
                in_range: day_was_in_range,
                cumulative_fees_usd: cumulative_fees,
                il_usd,
                net_pnl_usd,
            });

            // Reset day accumulators.
            current_day_date = day_date;
            day_was_in_range = false;
        }

        // Track whether this day saw any in-range tick.
        if in_range {
            day_was_in_range = true;
        }

        // ── Fee accrual ─────────────────────────────────────────────────────
        // Skip the very first tick (no prior reference for the delta).
        // Guard against division by zero (T-03-06).
        if i > 0 && t.liquidity > 0 {
            let delta = fee_growth_delta(prev_fg_a, t.fee_growth_global_a);
            // X64 fixed-point de-scaling: divide by 2^64.
            const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0_f64;
            let share = input.position_liquidity as f64 / t.liquidity as f64;
            let fees_step = (delta as f64 / TWO_POW_64) * share;
            cumulative_fees += fees_step;
        }
        prev_fg_a = t.fee_growth_global_a;

        // ── Rebalance signal ────────────────────────────────────────────────
        let il_frac = compute_il(cur_entry_price, price, cur_price_lower, cur_price_upper);
        let il_usd = il_frac * input.initial_value_usd;
        let net_pnl = cumulative_fees + il_usd;

        let signal = strategy::should_rebalance(
            t.tick_current,
            tick_lower,
            tick_upper,
            net_pnl,
            &input.rebalance_cfg,
        );

        if matches!(signal, RebalanceDecision::Rebalance { .. }) {
            total_rebalances += 1;
            // Re-centre range around current price using the configured factors.
            cur_entry_price = price;
            cur_price_lower = (price * input.range_factor_lower).max(1e-9);
            cur_price_upper = price * input.range_factor_upper;
            tick_lower = price_to_tick(cur_price_lower, input.tick_spacing);
            tick_upper = price_to_tick(cur_price_upper, input.tick_spacing);
        }
    }

    // ── Flush the final day ─────────────────────────────────────────────────
    let last = ticks.last().unwrap(); // safe: non-empty checked above
    let final_price = sqrt_q64_to_price(last.sqrt_price);
    let final_in_range = last.tick_current >= tick_lower && last.tick_current <= tick_upper;

    if final_in_range {
        day_was_in_range = true;
    }
    if day_was_in_range {
        days_in_range += 1;
    }

    let final_il_frac = compute_il(
        cur_entry_price,
        final_price,
        cur_price_lower,
        cur_price_upper,
    );
    let final_il_usd = final_il_frac * input.initial_value_usd;
    let final_net_pnl_usd = cumulative_fees + final_il_usd;

    current_day_num += 1;
    day_results.push(DayResult {
        day: current_day_num,
        price: final_price,
        in_range: day_was_in_range,
        cumulative_fees_usd: cumulative_fees,
        il_usd: final_il_usd,
        net_pnl_usd: final_net_pnl_usd,
    });

    // ── Fee APY ─────────────────────────────────────────────────────────────
    let total_days = current_day_num;
    let fee_apy_pct = if total_days > 0 && input.initial_value_usd > 0.0 {
        (cumulative_fees / input.initial_value_usd) * (365.0 / total_days as f64) * 100.0
    } else {
        0.0
    };

    Ok(BacktestResult {
        params_snapshot: ParamsSnapshot {
            entry_price: input.entry_price,
            price_lower: input.price_lower,
            price_upper: input.price_upper,
            fee_rate_bps: input.fee_rate_bps,
            annual_vol_pct: 0.0,   // not applicable in DB mode
            daily_volume_usd: 0.0, // not applicable in DB mode
            initial_value_usd: input.initial_value_usd,
        },
        days: day_results,
        total_fees_usd: cumulative_fees,
        total_il_usd: final_il_usd,
        net_pnl_usd: final_net_pnl_usd,
        total_rebalances,
        days_in_range,
        fee_apy_pct,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_tick(
        time_secs: i64,
        tick_current: i32,
        sqrt_price: u128,
        liquidity: u128,
        fee_growth_global_a: u128,
    ) -> PoolTickRow {
        PoolTickRow {
            time: Utc.timestamp_opt(time_secs, 0).unwrap(),
            pool_address: "TEST_POOL".to_string(),
            slot: time_secs,
            tick_current,
            sqrt_price,
            liquidity,
            fee_growth_global_a,
            fee_growth_global_b: 0,
        }
    }

    fn default_input() -> DbBacktestInput {
        DbBacktestInput {
            initial_value_usd: 10_000.0,
            entry_price: 1.0,
            price_lower: 0.90,
            price_upper: 1.10,
            fee_rate_bps: 5.0,
            tick_spacing: 64,
            position_liquidity: 100,
            rebalance_cfg: RebalanceConfig {
                rebalance_out_of_range: false,
                near_edge_ticks: 0,
                min_net_pnl_usd: 0.0,
            },
            range_factor_lower: 0.95,
            range_factor_upper: 1.05,
        }
    }

    // ── Wrapping / sqrt_price conversion ─────────────────────────────────────

    #[test]
    fn fee_growth_delta_wraps() {
        // u128::MAX wrapping_sub: MAX then 5 → delta = 6
        assert_eq!(fee_growth_delta(u128::MAX, 5), 6);
        assert_eq!(fee_growth_delta(100, 200), 100);
        assert_eq!(fee_growth_delta(0, 0), 0);
    }

    #[test]
    fn sqrt_price_conversion_matches_watch_loop() {
        // sqrt_price = 2^64 → price = 1.0
        let p = sqrt_q64_to_price(1u128 << 64);
        assert!((p - 1.0).abs() < 1e-9, "expected 1.0, got {}", p);
    }

    #[test]
    fn sqrt_price_zero_gives_zero() {
        assert_eq!(sqrt_q64_to_price(0), 0.0);
    }

    // ── Empty tick stream ────────────────────────────────────────────────────

    #[test]
    fn empty_ticks_returns_error() {
        let input = default_input();
        let result = run_db_backtest(input, &[]);
        assert!(result.is_err(), "expected Err for empty tick stream");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("empty"),
            "error should mention 'empty': {}",
            msg
        );
    }

    // ── Single tick (only day-0 flush) ───────────────────────────────────────

    #[test]
    fn single_tick_produces_one_day() {
        let sqrt_price_at_1 = 1u128 << 64; // price = 1.0
        let ticks = vec![make_tick(0, 0, sqrt_price_at_1, 1000, 0)];
        let input = default_input();
        let result = run_db_backtest(input, &ticks).unwrap();
        assert_eq!(result.days.len(), 1);
        assert_eq!(result.days[0].day, 1);
    }

    // ── Fee accrual correctness ───────────────────────────────────────────────

    #[test]
    fn fee_accrual_basic() {
        // Two ticks same day: fee_growth_global_a goes 100 → 200,
        // pool_liquidity = 1000, position_liquidity = 100 → share = 0.1
        // delta = 100, fees_step = (100 / 2^64) * 0.1
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1000, 100),
            make_tick(1, 0, sqrt_price_at_1, 1000, 200),
        ];
        let input = default_input(); // position_liquidity = 100
        let result = run_db_backtest(input, &ticks).unwrap();

        let expected_fees = (100_f64 / 18_446_744_073_709_551_616.0_f64) * 0.1;
        assert!(
            (result.total_fees_usd - expected_fees).abs() < 1e-20,
            "expected fees ~{:.2e}, got {:.2e}",
            expected_fees,
            result.total_fees_usd
        );
    }

    #[test]
    fn fee_accrual_with_wrapping() {
        // fee_growth_global_a wraps: prev = u128::MAX, curr = 5 → delta = 6
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1000, u128::MAX),
            make_tick(1, 0, sqrt_price_at_1, 1000, 5),
        ];
        let input = default_input(); // position_liquidity = 100
        let result = run_db_backtest(input, &ticks).unwrap();

        // delta = wrapping_sub(5, u128::MAX) = 6
        let expected_fees = (6_f64 / 18_446_744_073_709_551_616.0_f64) * 0.1;
        assert!(
            (result.total_fees_usd - expected_fees).abs() < 1e-30,
            "expected wrapping fees ~{:.2e}, got {:.2e}",
            expected_fees,
            result.total_fees_usd
        );
    }

    #[test]
    fn zero_liquidity_skips_fee_accrual() {
        // pool_liquidity = 0 → div-by-zero guard (T-03-06), fees stay 0
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 0, 100),
            make_tick(1, 0, sqrt_price_at_1, 0, 200),
        ];
        let input = default_input();
        let result = run_db_backtest(input, &ticks).unwrap();
        assert_eq!(
            result.total_fees_usd, 0.0,
            "zero pool liquidity must skip fee accrual"
        );
    }

    // ── Multi-day roll-up ────────────────────────────────────────────────────

    #[test]
    fn multi_day_produces_correct_day_count() {
        let sqrt_price_at_1 = 1u128 << 64;
        // 3 distinct UTC days: day 0, day 1, day 2
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1000, 0), // 1970-01-01
            make_tick(86400, 0, sqrt_price_at_1, 1000, 50), // 1970-01-02
            make_tick(2 * 86400, 0, sqrt_price_at_1, 1000, 100), // 1970-01-03
        ];
        let input = default_input();
        let result = run_db_backtest(input, &ticks).unwrap();
        // 3 distinct days → 3 DayResult entries
        assert_eq!(result.days.len(), 3, "expected 3 DayResult entries");
        assert_eq!(result.days[0].day, 1);
        assert_eq!(result.days[1].day, 2);
        assert_eq!(result.days[2].day, 3);
    }

    // ── Rebalance detection ───────────────────────────────────────────────────

    #[test]
    fn out_of_range_triggers_rebalance() {
        // tick_current = 9999 >> tick_upper computed from price 1.10 ≈ tick 952
        // With rebalance_out_of_range = true this should fire a rebalance.
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1000, 0),
            make_tick(1, 9999, sqrt_price_at_1, 1000, 0),
        ];
        let mut input = default_input();
        input.rebalance_cfg.rebalance_out_of_range = true;
        let result = run_db_backtest(input, &ticks).unwrap();
        assert!(
            result.total_rebalances > 0,
            "expected at least one rebalance"
        );
    }

    #[test]
    fn no_rebalances_when_disabled() {
        // rebalance_out_of_range = false and near_edge_ticks = i32::MIN (impossible
        // to satisfy) → no rebalance even with tick way out of range.
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1000, 0),
            make_tick(1, 9999, sqrt_price_at_1, 1000, 0),
        ];
        let mut input = default_input();
        // near_edge_ticks = i32::MIN prevents the near-edge branch from firing
        // even when the tick is far outside the range boundaries.
        input.rebalance_cfg.near_edge_ticks = i32::MIN;
        let result = run_db_backtest(input, &ticks).unwrap();
        assert_eq!(result.total_rebalances, 0);
    }

    // ── BacktestResult schema matches GBM mode ────────────────────────────────

    #[test]
    fn result_schema_has_expected_fields() {
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1000, 0),
            make_tick(1, 0, sqrt_price_at_1, 1000, 100),
        ];
        let input = default_input();
        let r = run_db_backtest(input, &ticks).unwrap();
        // All BacktestResult fields are accessible (same schema as GBM mode).
        let _ = r.days;
        let _ = r.total_fees_usd;
        let _ = r.total_il_usd;
        let _ = r.net_pnl_usd;
        let _ = r.total_rebalances;
        let _ = r.days_in_range;
        let _ = r.fee_apy_pct;
        let _ = r.params_snapshot;
    }

    // ── CLI-level fixture tests: verify DB-mode output on synthetic data ──────
    //
    // These tests use hand-crafted PoolTickRow fixtures that represent the same
    // conditions as a GBM run at zero volatility (price constant at entry,
    // always in range). They validate that the DB-mode path produces results
    // consistent with what the GBM simulator would compute for the same inputs.

    /// At zero volatility GBM keeps price fixed at entry; the equivalent DB fixture
    /// is a series of ticks where sqrt_price = 2^64 (price = 1.0) and
    /// tick_current = 0 (range [tick(0.90), tick(1.10)] includes 0).
    ///
    /// For DB mode the fee source is fee_growth_global_a deltas, so we use
    /// a large liquidity pool to make the per-tick fee contribution measurable.
    #[test]
    fn db_mode_in_range_constant_price_fees_positive() {
        // price = 1.0 throughout; position is always in range.
        let sqrt_price_at_1 = 1u128 << 64;
        // 3 ticks across 2 days; fee_growth_global_a grows by 1_000_000_000 per tick.
        let fg_step: u128 = 1_000_000_000;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 10_000, 0),
            make_tick(43200, 0, sqrt_price_at_1, 10_000, fg_step), // same day
            make_tick(86400, 0, sqrt_price_at_1, 10_000, 2 * fg_step), // day 2
        ];
        let input = DbBacktestInput {
            initial_value_usd: 10_000.0,
            entry_price: 1.0,
            price_lower: 0.90,
            price_upper: 1.10,
            fee_rate_bps: 5.0,
            tick_spacing: 64,
            position_liquidity: 100, // 100/10_000 = 1% of pool
            rebalance_cfg: RebalanceConfig {
                rebalance_out_of_range: false,
                near_edge_ticks: 0,
                min_net_pnl_usd: 0.0,
            },
            range_factor_lower: 0.95,
            range_factor_upper: 1.05,
        };
        let result = run_db_backtest(input, &ticks).unwrap();

        // Price is constant at entry → IL must be exactly 0.
        assert!(
            result.total_il_usd.abs() < 1e-9,
            "constant price → IL must be 0, got {}",
            result.total_il_usd
        );

        // Position was always in range → days_in_range == day count.
        assert_eq!(
            result.days_in_range,
            result.days.len() as u32,
            "all days should be in range"
        );

        // Fees must be positive (fee_growth_global_a grew).
        assert!(
            result.total_fees_usd > 0.0,
            "fees should be positive when fee_growth_global_a grows"
        );

        // net_pnl = fees + il = fees (since il = 0).
        let expected_net = result.total_fees_usd + result.total_il_usd;
        assert!(
            (result.net_pnl_usd - expected_net).abs() < 1e-9,
            "net_pnl_usd must equal fees + il"
        );

        // No rebalances (price never left range, rebalance disabled).
        assert_eq!(result.total_rebalances, 0);
    }

    /// GBM invariant: net_pnl = fees + il must hold at every day snapshot,
    /// mirroring the same check in `backtest::tests::net_pnl_equals_fees_plus_il_each_day`.
    #[test]
    fn db_mode_net_pnl_equals_fees_plus_il_each_day() {
        let sqrt_price_at_1 = 1u128 << 64;
        // 4 ticks across 3 distinct days.
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 5_000, 0),
            make_tick(86400, 0, sqrt_price_at_1, 5_000, 500_000),
            make_tick(86401, 0, sqrt_price_at_1, 5_000, 500_001),
            make_tick(172800, 0, sqrt_price_at_1, 5_000, 1_000_000),
        ];
        let input = DbBacktestInput {
            initial_value_usd: 10_000.0,
            entry_price: 1.0,
            price_lower: 0.90,
            price_upper: 1.10,
            fee_rate_bps: 5.0,
            tick_spacing: 64,
            position_liquidity: 50,
            rebalance_cfg: RebalanceConfig {
                rebalance_out_of_range: false,
                near_edge_ticks: 0,
                min_net_pnl_usd: 0.0,
            },
            range_factor_lower: 0.95,
            range_factor_upper: 1.05,
        };
        let result = run_db_backtest(input, &ticks).unwrap();

        for day in &result.days {
            let expected = day.cumulative_fees_usd + day.il_usd;
            assert!(
                (day.net_pnl_usd - expected).abs() < 1e-9,
                "day {}: net_pnl_usd ({}) ≠ fees_usd ({}) + il_usd ({})",
                day.day,
                day.net_pnl_usd,
                day.cumulative_fees_usd,
                day.il_usd
            );
        }
    }

    /// fee_apy_pct must be non-negative and finite for any valid tick stream.
    #[test]
    fn db_mode_fee_apy_is_non_negative_and_finite() {
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1_000, 0),
            make_tick(86400, 0, sqrt_price_at_1, 1_000, 200_000),
        ];
        let input = default_input();
        let result = run_db_backtest(input, &ticks).unwrap();
        assert!(
            result.fee_apy_pct >= 0.0,
            "fee_apy_pct must be non-negative"
        );
        assert!(result.fee_apy_pct.is_finite(), "fee_apy_pct must be finite");
    }

    /// Verify params_snapshot is populated from input (not from GBM params),
    /// confirming the shared display path gets the right data in DB mode.
    #[test]
    fn db_mode_params_snapshot_matches_input() {
        let sqrt_price_at_1 = 1u128 << 64;
        let ticks = vec![
            make_tick(0, 0, sqrt_price_at_1, 1_000, 0),
            make_tick(1, 0, sqrt_price_at_1, 1_000, 0),
        ];
        let input = DbBacktestInput {
            initial_value_usd: 50_000.0,
            entry_price: 2.5,
            price_lower: 2.0,
            price_upper: 3.0,
            fee_rate_bps: 10.0,
            tick_spacing: 8,
            position_liquidity: 200,
            rebalance_cfg: RebalanceConfig {
                rebalance_out_of_range: false,
                near_edge_ticks: 0,
                min_net_pnl_usd: 0.0,
            },
            range_factor_lower: 0.90,
            range_factor_upper: 1.10,
        };
        let result = run_db_backtest(input, &ticks).unwrap();
        let snap = &result.params_snapshot;

        assert!((snap.entry_price - 2.5).abs() < 1e-9);
        assert!((snap.price_lower - 2.0).abs() < 1e-9);
        assert!((snap.price_upper - 3.0).abs() < 1e-9);
        assert!((snap.fee_rate_bps - 10.0).abs() < 1e-9);
        assert!((snap.initial_value_usd - 50_000.0).abs() < 1e-9);
        // DB mode sets these to 0.0 (not applicable).
        assert_eq!(snap.annual_vol_pct, 0.0);
        assert_eq!(snap.daily_volume_usd, 0.0);
    }
}
