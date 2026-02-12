//! Backtest engine — simulates LP P&L over a synthetic price path.
//!
//! Generates a Geometric Brownian Motion price series from the current pool
//! state, then applies CLMM math at each step (IL, fee accrual).
//! Optionally fires rebalance events via the strategy signal module.

pub mod db_replay;

use crate::math::il::compute_il;
use crate::strategy::{self, RebalanceConfig, RebalanceDecision};

// ── Parameters ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct BacktestParams {
    /// Price when the position was opened (token A in token B units).
    pub entry_price: f64,
    /// Lower bound of the LP range.
    pub price_lower: f64,
    /// Upper bound of the LP range.
    pub price_upper: f64,
    /// Pool fee rate in basis points (e.g. 5.0 = 0.05%).
    pub fee_rate_bps: f64,
    /// Position value in USD at open.
    pub initial_value_usd: f64,
    /// Number of calendar days to simulate.
    pub days: u32,
    /// Annualised volatility of the underlying (e.g. 0.80 = 80%).
    pub annual_volatility: f64,
    /// Estimated daily volume through the pool in USD.
    pub daily_volume_usd: f64,
    /// Fraction of pool daily volume captured by this position (0.0–1.0). Typical narrow range: 0.05–0.30.
    pub position_volume_share: f64,
    /// Tick spacing (used to map prices back to integer ticks for the signal).
    pub tick_spacing: i32,
    /// Fire a rebalance when out of range; resets IL clock and range.
    pub strategy_rebalance: bool,
}

// ── Per-day snapshot ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct DayResult {
    pub day: u32,
    pub price: f64,
    pub in_range: bool,
    pub cumulative_fees_usd: f64,
    pub il_usd: f64,
    pub net_pnl_usd: f64,
}

// ── Aggregate result ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct BacktestResult {
    pub params_snapshot: ParamsSnapshot,
    pub days: Vec<DayResult>,
    pub total_fees_usd: f64,
    pub total_il_usd: f64,
    pub net_pnl_usd: f64,
    pub total_rebalances: u32,
    pub days_in_range: u32,
    pub fee_apy_pct: f64,
}

#[derive(Debug)]
pub struct ParamsSnapshot {
    pub entry_price: f64,
    pub price_lower: f64,
    pub price_upper: f64,
    pub fee_rate_bps: f64,
    pub annual_vol_pct: f64,
    pub daily_volume_usd: f64,
    pub initial_value_usd: f64,
}

// ── Minimal PRNG (LCG + Box-Muller) — no external deps ───────────────────────

struct Prng(u64);

impl Prng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0xdeadbeef_cafebabe)
    }

    fn next_u64(&mut self) -> u64 {
        // Knuth multiplicative hash (Knuth vol 2, §3.3.4)
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    /// Uniform (0, 1].
    fn next_f64(&mut self) -> f64 {
        let bits = self.next_u64();
        // Use top 53 bits for IEEE-754 double mantissa precision.
        ((bits >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// Standard normal via Box-Muller transform.
    fn next_normal(&mut self) -> f64 {
        let u1 = self.next_f64();
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

// ── Price-to-tick helper ──────────────────────────────────────────────────────

pub(crate) fn price_to_tick(price: f64, tick_spacing: i32) -> i32 {
    // tick = log_{1.0001}(price) = ln(price) / ln(1.0001)
    let raw = (price.ln() / 1.0001_f64.ln()).floor() as i32;
    // Round down to nearest multiple of tick_spacing (floor division for negative ticks).
    raw.div_euclid(tick_spacing) * tick_spacing
}

// ── Core simulation ───────────────────────────────────────────────────────────

pub fn run(params: &BacktestParams, seed: u64) -> BacktestResult {
    let mut rng = Prng::new(seed);

    // GBM parameters.
    let dt = 1.0 / 365.0;
    let sigma = params.annual_volatility;
    let drift = -0.5 * sigma * sigma * dt; // log-drift correction (μ = 0)
    let vol_step = sigma * dt.sqrt();

    let fee_fraction = params.fee_rate_bps / 10_000.0;

    let mut price = params.entry_price;
    let mut entry_price = params.entry_price;
    let mut price_lower = params.price_lower;
    let mut price_upper = params.price_upper;

    let mut cumulative_fees = 0.0_f64;
    let mut realized_il_usd = 0.0_f64;
    let mut total_rebalances: u32 = 0;
    let mut days_in_range: u32 = 0;
    let mut day_results: Vec<DayResult> = Vec::with_capacity(params.days as usize);

    let rebalance_cfg = RebalanceConfig {
        rebalance_out_of_range: params.strategy_rebalance,
        near_edge_ticks: 0, // only trigger on full out-of-range in backtest
        min_net_pnl_usd: 0.0,
    };

    for day in 1..=params.days {
        // GBM step: P_{t+1} = P_t * exp(drift + vol * Z)
        let z = rng.next_normal();
        price *= (drift + vol_step * z).exp();
        price = price.max(1e-6); // guard against degenerate paths

        let in_range = price >= price_lower && price <= price_upper;
        if in_range {
            days_in_range += 1;
        }

        // Fees: only accrue when in range.
        let fees_today = if in_range {
            params.daily_volume_usd * params.position_volume_share * fee_fraction
        } else {
            0.0
        };
        cumulative_fees += fees_today;

        // IL relative to current entry.
        let il_fraction = compute_il(entry_price, price, price_lower, price_upper);
        let position_value = params.initial_value_usd; // constant-liquidity approximation
        let il_usd = il_fraction * position_value;

        let net_pnl = cumulative_fees + il_usd;

        // Rebalance signal.
        if params.strategy_rebalance {
            let tick_current = price_to_tick(price, params.tick_spacing);
            let tick_lower = price_to_tick(price_lower, params.tick_spacing);
            let tick_upper = price_to_tick(price_upper, params.tick_spacing);

            let signal = strategy::should_rebalance(
                tick_current,
                tick_lower,
                tick_upper,
                net_pnl,
                &rebalance_cfg,
            );

            if matches!(signal, RebalanceDecision::Rebalance { .. }) {
                total_rebalances += 1;
                // Accumulate IL realized at this rebalance before resetting.
                realized_il_usd += il_usd;
                // Reset: re-center range around current price (same width).
                let half_width = (price_upper - price_lower) / 2.0;
                entry_price = price;
                price_lower = (price - half_width).max(1e-6);
                price_upper = price + half_width;
                // IL clock resets; fees continue accumulating.
            }
        }

        day_results.push(DayResult {
            day,
            price,
            in_range,
            cumulative_fees_usd: cumulative_fees,
            il_usd,
            net_pnl_usd: net_pnl,
        });
    }

    // Final totals from last day snapshot.
    let last = day_results.last();
    let total_fees = last.map(|d| d.cumulative_fees_usd).unwrap_or(0.0);
    let last_day_il = last.map(|d| d.il_usd).unwrap_or(0.0);
    let total_il = realized_il_usd + last_day_il;
    let net = total_fees + total_il;

    let fee_apy = if params.days > 0 && params.initial_value_usd > 0.0 {
        (total_fees / params.initial_value_usd) * (365.0 / params.days as f64) * 100.0
    } else {
        0.0
    };

    BacktestResult {
        params_snapshot: ParamsSnapshot {
            entry_price: params.entry_price,
            price_lower: params.price_lower,
            price_upper: params.price_upper,
            fee_rate_bps: params.fee_rate_bps,
            annual_vol_pct: params.annual_volatility * 100.0,
            daily_volume_usd: params.daily_volume_usd,
            initial_value_usd: params.initial_value_usd,
        },
        days: day_results,
        total_fees_usd: total_fees,
        total_il_usd: total_il,
        net_pnl_usd: net,
        total_rebalances,
        days_in_range,
        fee_apy_pct: fee_apy,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn base_params() -> BacktestParams {
        BacktestParams {
            entry_price: 100.0,
            price_lower: 90.0,
            price_upper: 110.0,
            fee_rate_bps: 5.0,
            initial_value_usd: 10_000.0,
            days: 30,
            annual_volatility: 0.80,
            daily_volume_usd: 1_000_000.0,
            position_volume_share: 0.05,
            tick_spacing: 64,
            strategy_rebalance: false,
        }
    }

    // ── Determinism ───────────────────────────────────────────────────────────

    #[test]
    fn run_is_deterministic() {
        let params = base_params();
        let r1 = run(&params, 42);
        let r2 = run(&params, 42);
        assert_eq!(r1.total_fees_usd, r2.total_fees_usd);
        assert_eq!(r1.total_il_usd, r2.total_il_usd);
        assert_eq!(r1.days_in_range, r2.days_in_range);
        assert_eq!(r1.net_pnl_usd, r2.net_pnl_usd);
    }

    #[test]
    fn different_seeds_produce_different_paths() {
        let params = base_params();
        let r1 = run(&params, 1);
        let r2 = run(&params, 2);
        // Two 30-day paths at 80% vol will virtually always diverge.
        assert_ne!(
            r1.net_pnl_usd, r2.net_pnl_usd,
            "distinct seeds should yield distinct paths"
        );
    }

    // ── Zero-volatility invariants ────────────────────────────────────────────

    #[test]
    fn zero_volatility_price_stays_at_entry() {
        let mut params = base_params();
        params.annual_volatility = 0.0;
        let result = run(&params, 0);
        for day in &result.days {
            // With σ=0: drift = 0, vol_step = 0 → price = entry * exp(0) = entry
            assert!(
                (day.price - params.entry_price).abs() < 1e-6,
                "day {}: price {} should equal entry {}",
                day.day,
                day.price,
                params.entry_price,
            );
        }
    }

    #[test]
    fn zero_volatility_all_days_in_range() {
        let mut params = base_params();
        params.annual_volatility = 0.0;
        let result = run(&params, 0);
        assert_eq!(
            result.days_in_range, params.days,
            "all days should be in range with zero vol"
        );
    }

    #[test]
    fn zero_volatility_fees_monotone() {
        let mut params = base_params();
        params.annual_volatility = 0.0;
        let result = run(&params, 0);
        for w in result.days.windows(2) {
            assert!(
                w[1].cumulative_fees_usd >= w[0].cumulative_fees_usd,
                "cumulative fees must not decrease"
            );
        }
    }

    // ── Accounting identities ─────────────────────────────────────────────────

    #[test]
    fn net_pnl_equals_fees_plus_il_each_day() {
        let params = base_params();
        let result = run(&params, 42);
        for day in &result.days {
            let expected = day.cumulative_fees_usd + day.il_usd;
            assert!(
                (day.net_pnl_usd - expected).abs() < 1e-9,
                "day {}: net_pnl_usd ({}) ≠ fees + il ({})",
                day.day,
                day.net_pnl_usd,
                expected,
            );
        }
    }

    #[test]
    fn total_fees_matches_last_day_cumulative() {
        let params = base_params();
        let result = run(&params, 42);
        if let Some(last) = result.days.last() {
            assert_eq!(result.total_fees_usd, last.cumulative_fees_usd);
        }
    }

    // ── Structural invariants ─────────────────────────────────────────────────

    #[test]
    fn correct_number_of_day_snapshots() {
        let params = base_params();
        let result = run(&params, 0);
        assert_eq!(result.days.len(), params.days as usize);
        assert_eq!(result.days.first().map(|d| d.day), Some(1));
        assert_eq!(result.days.last().map(|d| d.day), Some(params.days));
    }

    #[test]
    fn zero_days_returns_empty_result() {
        let mut params = base_params();
        params.days = 0;
        let result = run(&params, 0);
        assert!(result.days.is_empty());
        assert_eq!(result.total_fees_usd, 0.0);
        assert_eq!(result.fee_apy_pct, 0.0);
        assert_eq!(result.days_in_range, 0);
    }

    #[test]
    fn days_in_range_bounded_by_total_days() {
        let params = base_params();
        let result = run(&params, 42);
        assert!(result.days_in_range <= params.days);
    }

    // ── Fee APY formula ───────────────────────────────────────────────────────

    #[test]
    fn fee_apy_correct_for_all_in_range_one_year() {
        let mut params = base_params();
        params.annual_volatility = 0.0; // price fixed at entry → always in range
        params.days = 365;
        let result = run(&params, 0);

        // daily_fee = volume * share * rate = 1_000_000 * 0.05 * (5/10_000) = 25.0
        let daily_fee =
            params.daily_volume_usd * params.position_volume_share * params.fee_rate_bps / 10_000.0;
        let expected_total = daily_fee * 365.0;
        let expected_apy = (expected_total / params.initial_value_usd) * 100.0; // 91.25 %

        assert!(
            (result.fee_apy_pct - expected_apy).abs() < 0.01,
            "expected APY ≈{:.2}%, got {:.2}%",
            expected_apy,
            result.fee_apy_pct,
        );
    }

    // ── Rebalance ─────────────────────────────────────────────────────────────

    #[test]
    fn rebalance_count_bounded_by_total_days() {
        let mut params = base_params();
        params.strategy_rebalance = true;
        params.annual_volatility = 5.0; // extreme vol → many out-of-range events
        params.days = 60;
        let result = run(&params, 7);
        assert!(
            result.total_rebalances <= result.days.len() as u32,
            "cannot rebalance more than once per simulated day"
        );
    }

    #[test]
    fn no_rebalances_without_strategy_flag() {
        let mut params = base_params();
        params.strategy_rebalance = false;
        params.annual_volatility = 5.0; // would trigger rebalances if enabled
        let result = run(&params, 7);
        assert_eq!(result.total_rebalances, 0);
    }

    // ── price_to_tick ─────────────────────────────────────────────────────────

    #[test]
    fn price_to_tick_at_price_one() {
        // ln(1.0) = 0 → tick = 0 regardless of tick_spacing
        assert_eq!(price_to_tick(1.0, 1), 0);
        assert_eq!(price_to_tick(1.0, 64), 0);
    }

    #[test]
    fn price_to_tick_is_monotone() {
        let prices = [0.001_f64, 0.1, 1.0, 10.0, 1_000.0, 100_000.0];
        let ticks: Vec<i32> = prices.iter().map(|&p| price_to_tick(p, 1)).collect();
        for w in ticks.windows(2) {
            assert!(
                w[1] >= w[0],
                "ticks must be non-decreasing with price: {:?}",
                ticks
            );
        }
    }

    #[test]
    fn price_to_tick_aligned_to_spacing() {
        for spacing in [1, 8, 64] {
            for price in [0.5_f64, 1.0, 2.0, 100.0, 10_000.0] {
                let tick = price_to_tick(price, spacing);
                assert_eq!(
                    tick % spacing,
                    0,
                    "tick {} not aligned to spacing {} for price {}",
                    tick,
                    spacing,
                    price,
                );
            }
        }
    }

    #[test]
    fn price_to_tick_below_one_is_negative() {
        let tick = price_to_tick(0.5, 1);
        assert!(
            tick < 0,
            "price below 1.0 should map to negative tick, got {}",
            tick
        );
    }

    // ── Proptest invariants ───────────────────────────────────────────────────

    proptest! {
        #[test]
        fn prop_days_in_range_leq_total_days(seed: u64, days in 1u32..=90u32) {
            let mut params = base_params();
            params.days = days;
            let result = run(&params, seed);
            prop_assert!(result.days_in_range <= days);
        }

        #[test]
        fn prop_cumulative_fees_nonnegative(seed: u64, days in 1u32..=90u32) {
            let mut params = base_params();
            params.days = days;
            let result = run(&params, seed);
            for day in &result.days {
                prop_assert!(day.cumulative_fees_usd >= 0.0);
            }
        }

        #[test]
        fn prop_net_pnl_equals_fees_plus_il(seed: u64, days in 1u32..=30u32) {
            let mut params = base_params();
            params.days = days;
            let result = run(&params, seed);
            for day in &result.days {
                let expected = day.cumulative_fees_usd + day.il_usd;
                prop_assert!((day.net_pnl_usd - expected).abs() < 1e-6);
            }
        }

        #[test]
        fn prop_price_to_tick_spacing_aligned(price_exp in -4i32..=5i32, spacing in prop::sample::select(vec![1i32, 8, 64])) {
            let price = 10.0_f64.powi(price_exp);
            let tick = price_to_tick(price, spacing);
            prop_assert_eq!(tick % spacing, 0, "tick {} not aligned to spacing {}", tick, spacing);
        }
    }
}

// ── Display ───────────────────────────────────────────────────────────────────

pub fn print_results(result: &BacktestResult) {
    let p = &result.params_snapshot;
    let n = result.days.len() as u32;

    println!("Backtest — CLMM LP Simulation");
    println!("{}", "─".repeat(60));
    println!(
        "Entry:         ${:.4}   Range: ${:.4} – ${:.4}",
        p.entry_price, p.price_lower, p.price_upper
    );
    println!(
        "Fee:           {:.0} bps   Vol: {:.0}% ann.   Volume: ${:.0}/day",
        p.fee_rate_bps, p.annual_vol_pct, p.daily_volume_usd
    );
    println!("Capital:       ${:.0}   Days: {}", p.initial_value_usd, n);
    println!("{}", "─".repeat(60));

    // Sample up to 10 evenly-spaced rows so the table isn't overwhelming.
    let sample_days: Vec<usize> = if n <= 10 {
        (0..n as usize).collect()
    } else {
        let mut v: Vec<usize> = (0..10).map(|i| i * (n as usize - 1) / 9).collect();
        v.dedup();
        v
    };

    println!(
        "{:>4}  {:>10}  {:>8}  {:>10}  {:>10}  {:>10}",
        "Day", "Price", "InRange", "CumFees", "IL", "NetP&L"
    );
    println!("{}", "─".repeat(60));

    for &idx in &sample_days {
        if idx >= result.days.len() {
            continue;
        }
        let d = &result.days[idx];
        println!(
            "{:>4}  {:>10.4}  {:>8}  {:>10.2}  {:>10.2}  {:>10.2}",
            d.day,
            d.price,
            if d.in_range { "yes" } else { "NO" },
            d.cumulative_fees_usd,
            d.il_usd,
            d.net_pnl_usd,
        );
    }

    println!("{}", "═".repeat(60));
    let fees_pct = if p.initial_value_usd > 0.0 {
        result.total_fees_usd / p.initial_value_usd * 100.0
    } else {
        0.0
    };
    let il_pct = if p.initial_value_usd > 0.0 {
        result.total_il_usd / p.initial_value_usd * 100.0
    } else {
        0.0
    };
    let net_pct = if p.initial_value_usd > 0.0 {
        result.net_pnl_usd / p.initial_value_usd * 100.0
    } else {
        0.0
    };
    println!(
        "Fees earned:   ${:.2}  ({:.1}% of capital)",
        result.total_fees_usd, fees_pct,
    );
    println!(
        "Imperm. loss:  ${:.2}  ({:.1}% of capital)",
        result.total_il_usd, il_pct,
    );
    println!(
        "Net P&L:       ${:.2}  ({:.1}% of capital)",
        result.net_pnl_usd, net_pct,
    );
    println!("Fee APY:       {:.1}%", result.fee_apy_pct);
    println!(
        "Days in range: {}/{} ({:.0}%)",
        result.days_in_range,
        n,
        result.days_in_range as f64 / n as f64 * 100.0
    );
    if result.total_rebalances > 0 {
        println!("Rebalances:    {}", result.total_rebalances);
    }
}
