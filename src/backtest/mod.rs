//! Backtest engine — simulates LP P&L over a synthetic price path.
//!
//! Generates a Geometric Brownian Motion price series from the current pool
//! state, then applies CLMM math at each step (IL, fee accrual).
//! Optionally fires rebalance events via the strategy signal module.

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

fn price_to_tick(price: f64, tick_spacing: i32) -> i32 {
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
    println!(
        "Capital:       ${:.0}   Days: {}",
        p.initial_value_usd, n
    );
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
    let fees_pct = if p.initial_value_usd > 0.0 { result.total_fees_usd / p.initial_value_usd * 100.0 } else { 0.0 };
    let il_pct = if p.initial_value_usd > 0.0 { result.total_il_usd / p.initial_value_usd * 100.0 } else { 0.0 };
    let net_pct = if p.initial_value_usd > 0.0 { result.net_pnl_usd / p.initial_value_usd * 100.0 } else { 0.0 };
    println!(
        "Fees earned:   ${:.2}  ({:.1}% of capital)",
        result.total_fees_usd,
        fees_pct,
    );
    println!(
        "Imperm. loss:  ${:.2}  ({:.1}% of capital)",
        result.total_il_usd,
        il_pct,
    );
    println!(
        "Net P&L:       ${:.2}  ({:.1}% of capital)",
        result.net_pnl_usd,
        net_pct,
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
