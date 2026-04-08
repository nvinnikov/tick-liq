//! Historical backtest engine for rebalance strategies.
//!
//! Replays a sequence of `(timestamp, price, fee_quote_delta)` observations
//! through a [`SignalEngine`] and a [`RangeStrategy`], producing a
//! [`BacktestReport`] with total fees, total IL, net P&L, the number of
//! rebalances, and the max drawdown.
//!
//! ## Design
//!
//! The engine is a pure in-memory loop:
//!
//! 1. Open an initial LP position around `tick[0].price` using the
//!    configured `RangeStrategy`, seeded with a caller-supplied initial
//!    deposit (in base + quote token units).
//! 2. For each tick:
//!    - Accumulate fee delta into the window and lifetime counters.
//!    - Compute current LP composition and `V_lp_now` vs `V_hodl_now` at
//!      the tick price using the amounts formulas.
//!    - Build a [`MarketTick`] with the current snapshot and pass it to
//!      [`SignalEngine::on_tick`].
//!    - On `Rebalance`, "close" the position (compute final composition
//!      at the rebalance price), mark the rebalance, then "re-open" a
//!      new position around the rebalance price using the strategy.
//!      Position state is reset: entry composition, fee window, etc.
//! 3. Track the running `net_pnl` (cumulative fees + cumulative
//!    close-time IL across all position generations) and its peak-to-
//!    trough drawdown.
//!
//! IL is accumulated as "realized" each time a position closes: the
//! `V_lp_now − V_hodl_now` delta at the close price is locked in, and
//! the next position starts fresh. Between rebalances, *unrealized* IL
//! is still reflected in drawdown via the running-net computation.
//!
//! ## What this engine does NOT model
//!
//! - Swap slippage on rebalance.
//! - Transaction costs (gas) — add a flat cost-per-rebalance if you
//!   need that later, it's one subtraction.
//! - Tick-by-tick fee attribution — fee amounts come from the input
//!   stream; the engine does not compute them from volume.
//! - Time-in-range decay on fees when the price is outside the range.
//!   The input fees are assumed to already reflect reality.

use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

use crate::math::tick::{tick_to_price, TICK_BASE};
use crate::strategy::pnl::{compute_pnl, FeeDelta, PnlInput, PnlSnapshot};
use crate::strategy::range::{RangeContext, RangeRecommendation, RangeStrategy};
use crate::strategy::signal::{MarketTick, RebalanceSignal, SignalConfig, SignalEngine};

/// One historical observation fed to the backtest engine.
#[derive(Debug, Clone, Copy)]
pub struct BacktestTick {
    pub timestamp_secs: u64,
    pub price: f64,
    /// Fees in quote units earned since the previous tick (or since
    /// position open for the first tick). Non-negative.
    pub fee_quote_delta: f64,
}

/// Static backtest configuration.
#[derive(Debug, Clone, Copy)]
pub struct BacktestConfig {
    /// Signal engine thresholds.
    pub signal: SignalConfig,
    /// Context needed to call the range strategy. `current_price` inside
    /// this field is a placeholder — the engine overrides it on every
    /// (re)open with the actual price.
    pub range_ctx: RangeContext,
    /// Initial base-token amount deposited into the first position.
    pub initial_base: f64,
    /// Initial quote-token amount deposited into the first position.
    pub initial_quote: f64,
}

/// Aggregate result of a backtest run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacktestReport {
    /// Sum of all fee quote deltas across all position generations.
    pub total_fees: f64,
    /// Cumulative realized IL at position-close events. Non-positive.
    pub total_il_quote: f64,
    /// `total_fees + total_il_quote` (the realized edge vs HODL).
    pub net_pnl: f64,
    /// Number of rebalance events (position closes triggered by the
    /// signal engine, not the initial open and not the final close).
    pub num_rebalances: u32,
    /// Maximum drawdown of the running (fees + unrealized IL) path,
    /// expressed as a non-positive quote number: `min(running - peak)`.
    pub max_drawdown: f64,
}

/// Internal mutable state for an active position during the replay.
struct ActivePosition {
    entry_price: f64,
    lower_price: f64,
    upper_price: f64,
    entry_x: f64,
    entry_y: f64,
    window_fees: f64,
}

impl ActivePosition {
    /// Open a new position around `price` with the given deposit
    /// composition. Uses the proxy-liquidity formulation from `math::il`
    /// (amounts per unit liquidity) inverted to match the deposit value.
    fn open(price: f64, lower_price: f64, upper_price: f64, base: f64, quote: f64) -> Self {
        Self {
            entry_price: price,
            lower_price,
            upper_price,
            entry_x: base,
            entry_y: quote,
            window_fees: 0.0,
        }
    }

    /// P&L at the current tick (using window fees, not lifetime).
    fn pnl(&self, price: f64) -> Result<PnlSnapshot> {
        compute_pnl(PnlInput {
            entry_price: self.entry_price,
            current_price: price,
            lower_price: self.lower_price,
            upper_price: self.upper_price,
            entry_x: self.entry_x,
            entry_y: self.entry_y,
            fees: FeeDelta {
                base: 0.0,
                quote: self.window_fees,
            },
        })
    }
}

/// Run a backtest over `ticks`.
///
/// # Errors
/// Returns an error on empty `ticks`, non-monotonic timestamps, or any
/// downstream failure from the signal engine, range strategy, or P&L
/// computation.
pub fn run_backtest<S: RangeStrategy>(
    ticks: &[BacktestTick],
    strategy: &S,
    cfg: BacktestConfig,
) -> Result<BacktestReport> {
    if ticks.is_empty() {
        bail!("backtest requires at least one tick");
    }
    validate_ticks(ticks)?;

    let mut engine = SignalEngine::new(cfg.signal);

    // --- Initial open at tick[0] ---
    let first = ticks[0];
    let mut range_ctx = cfg.range_ctx;
    range_ctx.current_price = first.price;
    let rec = strategy.recommend(&range_ctx)?;
    let (lower_price, upper_price) =
        ticks_to_prices(rec, range_ctx.decimals_a, range_ctx.decimals_b)?;
    let mut position = ActivePosition::open(
        first.price,
        lower_price,
        upper_price,
        cfg.initial_base,
        cfg.initial_quote,
    );
    engine.set_target_range(rec);

    // --- Aggregates ---
    let mut total_fees: f64 = 0.0;
    let mut total_il_quote: f64 = 0.0;
    let mut num_rebalances: u32 = 0;
    let mut running_peak: f64 = 0.0;
    let mut max_drawdown: f64 = 0.0;

    for (i, t) in ticks.iter().enumerate() {
        // Fee accrual for this tick — the first tick's fee delta is
        // included in the first window.
        position.window_fees += t.fee_quote_delta;
        total_fees += t.fee_quote_delta;

        // Current per-tick P&L of the active position.
        let snap = position.pnl(t.price)?;

        // Track drawdown on the running (realized IL + lifetime fees +
        // unrealized IL of current position) path.
        let running_net = total_fees + total_il_quote + snap.il_quote;
        if running_net > running_peak {
            running_peak = running_net;
        }
        let dd = running_net - running_peak;
        if dd < max_drawdown {
            max_drawdown = dd;
        }

        // Feed the signal engine. We pre-recommend the next target range
        // using the current price so the engine has a valid payload ready.
        range_ctx.current_price = t.price;
        let next_rec = strategy.recommend(&range_ctx)?;
        engine.set_target_range(next_rec);

        let mtick = MarketTick {
            timestamp_secs: t.timestamp_secs,
            current_price: t.price,
            lower_price: position.lower_price,
            upper_price: position.upper_price,
            pnl: snap,
            fees_earned_quote: position.window_fees,
            manual_request: false,
        };
        let signal = engine.on_tick(mtick)?;

        // On rebalance: realize IL at the rebalance price, close the
        // position, open a new one. Skip rebalance on the very last tick
        // — closing at the terminal tick is handled in the final step.
        if let RebalanceSignal::Rebalance { .. } = signal {
            if i + 1 < ticks.len() {
                total_il_quote += snap.il_quote;
                num_rebalances += 1;

                let (new_lower, new_upper) =
                    ticks_to_prices(next_rec, range_ctx.decimals_a, range_ctx.decimals_b)?;
                // Re-open: reuse the current V_lp_now composition as the
                // deposit for the new position. This is the correct
                // "close and immediately redeploy" semantics.
                let (x_now, y_now) = amounts_at(&position, t.price);
                position = ActivePosition::open(t.price, new_lower, new_upper, x_now, y_now);
                engine.on_rebalance_executed(t.timestamp_secs);
            }
        }
    }

    // Final close at the terminal tick: realize whatever IL remains.
    let last = ticks[ticks.len() - 1];
    let final_snap = position.pnl(last.price)?;
    total_il_quote += final_snap.il_quote;

    let net_pnl = total_fees + total_il_quote;

    Ok(BacktestReport {
        total_fees,
        total_il_quote,
        net_pnl,
        num_rebalances,
        max_drawdown,
    })
}

fn validate_ticks(ticks: &[BacktestTick]) -> Result<()> {
    let mut prev: Option<u64> = None;
    for (i, t) in ticks.iter().enumerate() {
        if !t.price.is_finite() || t.price <= 0.0 {
            bail!(
                "tick[{}] price must be finite and positive, got {}",
                i,
                t.price
            );
        }
        if !t.fee_quote_delta.is_finite() || t.fee_quote_delta < 0.0 {
            bail!(
                "tick[{}] fee_quote_delta must be finite and non-negative, got {}",
                i,
                t.fee_quote_delta
            );
        }
        if let Some(p) = prev {
            if t.timestamp_secs < p {
                bail!("tick[{}] timestamp goes backwards", i);
            }
        }
        prev = Some(t.timestamp_secs);
    }
    Ok(())
}

/// Convert a `RangeRecommendation` into display-unit `(lower_price,
/// upper_price)` using the configured decimals.
fn ticks_to_prices(rec: RangeRecommendation, decimals_a: u8, decimals_b: u8) -> Result<(f64, f64)> {
    let lower = tick_to_price(rec.lower_tick, decimals_a, decimals_b)?;
    let upper = tick_to_price(rec.upper_tick, decimals_a, decimals_b)?;
    if lower >= upper {
        return Err(anyhow!(
            "range recommendation yields invalid prices: lower={}, upper={}",
            lower,
            upper
        ));
    }
    Ok((lower, upper))
}

/// Current composition `(x, y)` of `pos` at `price`, preserving the
/// initial deposit's total value proportionally. Used when closing a
/// position and redeploying into the next one.
fn amounts_at(pos: &ActivePosition, price: f64) -> (f64, f64) {
    // The amounts_per_unit_liquidity logic lives in math::il as a private
    // helper; we inline the Uniswap V3 formulas here against the position's
    // range, scaling by the position's entry value in quote so total value
    // is preserved modulo the IL delta.
    let sa = pos.lower_price.sqrt();
    let sb = pos.upper_price.sqrt();
    let s = price.sqrt();

    // Per-unit-liquidity amounts at entry and now.
    let (xe_pu, ye_pu) = per_l_amounts(pos.entry_price.sqrt(), sa, sb);
    let (x_pu, y_pu) = per_l_amounts(s, sa, sb);

    // Solve for the liquidity scale L such that at entry:
    //   L * xe_pu = entry_x and L * ye_pu = entry_y
    // Use whichever side has the larger magnitude to avoid division by
    // near-zero (happens when entry is at a range boundary).
    let l = if xe_pu * pos.entry_price >= ye_pu {
        if xe_pu > 0.0 {
            pos.entry_x / xe_pu
        } else {
            pos.entry_y / ye_pu
        }
    } else if ye_pu > 0.0 {
        pos.entry_y / ye_pu
    } else {
        pos.entry_x / xe_pu
    };

    (l * x_pu, l * y_pu)
}

fn per_l_amounts(s: f64, sa: f64, sb: f64) -> (f64, f64) {
    if s <= sa {
        (1.0 / sa - 1.0 / sb, 0.0)
    } else if s >= sb {
        (0.0, sb - sa)
    } else {
        (1.0 / s - 1.0 / sb, s - sa)
    }
}

/// Load a minimal CSV fixture with columns `timestamp_secs,price,fee_quote_delta`.
/// Lines starting with `#` and blank lines are ignored. A header row is
/// auto-detected (first column not parseable as u64).
pub fn load_ticks_csv<P: AsRef<Path>>(path: P) -> Result<Vec<BacktestTick>> {
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("reading backtest CSV at {:?}", path.as_ref()))?;
    let mut out = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split(',').map(str::trim);
        let a = it
            .next()
            .ok_or_else(|| anyhow!("line {}: empty", lineno + 1))?;
        let b = it
            .next()
            .ok_or_else(|| anyhow!("line {}: missing price", lineno + 1))?;
        let c = it
            .next()
            .ok_or_else(|| anyhow!("line {}: missing fee_quote_delta", lineno + 1))?;

        // Auto-skip header: if the first column does not parse as u64
        // and this is the first row, treat it as a header.
        let ts: u64 = match a.parse() {
            Ok(v) => v,
            Err(_) if out.is_empty() => continue,
            Err(e) => return Err(anyhow!("line {}: bad timestamp '{}': {}", lineno + 1, a, e)),
        };
        let price: f64 = b
            .parse()
            .with_context(|| format!("line {}: bad price '{}'", lineno + 1, b))?;
        let fee_quote_delta: f64 = c
            .parse()
            .with_context(|| format!("line {}: bad fee delta '{}'", lineno + 1, c))?;

        out.push(BacktestTick {
            timestamp_secs: ts,
            price,
            fee_quote_delta,
        });
    }
    if out.is_empty() {
        bail!("no data rows in CSV");
    }
    Ok(out)
}

// Keep TICK_BASE referenced so the dependency stays explicit in this
// module (tick math is a transitive requirement via ticks_to_prices).
const _: f64 = TICK_BASE;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::range::FixedWidth;
    use std::time::Duration;

    fn base_cfg() -> BacktestConfig {
        BacktestConfig {
            signal: SignalConfig {
                min_out_of_range: Duration::from_secs(60),
                pnl_loss_threshold_quote: f64::INFINITY,
                min_fees_per_day_quote: 0.0,
                fee_window_min: Duration::from_secs(3600),
            },
            range_ctx: RangeContext {
                current_price: 100.0,
                volatility: 0.05,
                tick_spacing: 64,
                decimals_a: 9,
                decimals_b: 6,
            },
            initial_base: 1.0,
            initial_quote: 100.0,
        }
    }

    fn flat_ticks(n: usize, price: f64, fee_per_tick: f64) -> Vec<BacktestTick> {
        (0..n)
            .map(|i| BacktestTick {
                timestamp_secs: i as u64 * 60,
                price,
                fee_quote_delta: fee_per_tick,
            })
            .collect()
    }

    #[test]
    fn flat_price_no_rebalance() {
        let ticks = flat_ticks(100, 100.0, 0.1);
        let strategy = FixedWidth {
            half_width_frac: 0.1,
        };
        let report = run_backtest(&ticks, &strategy, base_cfg()).unwrap();
        assert_eq!(report.num_rebalances, 0);
        // Fees accumulated deterministically:
        assert!((report.total_fees - 100.0 * 0.1).abs() < 1e-9);
        // No price move → zero IL.
        assert!(report.total_il_quote.abs() < 1e-6);
        assert!((report.net_pnl - report.total_fees).abs() < 1e-6);
    }

    #[test]
    fn big_price_move_triggers_rebalance() {
        // 30 ticks at 100 (all in range), then 30 ticks at 130 (out of
        // range, will fire OutOfRange after 60 s).
        let mut ticks: Vec<_> = (0..30)
            .map(|i| BacktestTick {
                timestamp_secs: i as u64 * 60,
                price: 100.0,
                fee_quote_delta: 0.1,
            })
            .collect();
        ticks.extend((0..30).map(|i| BacktestTick {
            timestamp_secs: (30 + i) as u64 * 60,
            price: 130.0,
            fee_quote_delta: 0.0,
        }));

        let strategy = FixedWidth {
            half_width_frac: 0.05,
        };
        let report = run_backtest(&ticks, &strategy, base_cfg()).unwrap();
        assert!(report.num_rebalances >= 1);
        // Some IL was realized on the big move.
        assert!(report.total_il_quote < 0.0);
        // Drawdown is non-positive.
        assert!(report.max_drawdown <= 0.0);
    }

    #[test]
    fn rejects_empty_and_bad_input() {
        let strategy = FixedWidth {
            half_width_frac: 0.1,
        };
        assert!(run_backtest(&[], &strategy, base_cfg()).is_err());

        let bad = vec![BacktestTick {
            timestamp_secs: 0,
            price: -1.0,
            fee_quote_delta: 0.0,
        }];
        assert!(run_backtest(&bad, &strategy, base_cfg()).is_err());

        let bad_ts = vec![
            BacktestTick {
                timestamp_secs: 100,
                price: 100.0,
                fee_quote_delta: 0.0,
            },
            BacktestTick {
                timestamp_secs: 50,
                price: 100.0,
                fee_quote_delta: 0.0,
            },
        ];
        assert!(run_backtest(&bad_ts, &strategy, base_cfg()).is_err());
    }

    #[test]
    fn csv_loader_parses_fixture() {
        let ticks = load_ticks_csv("tests/fixtures/backtest_sample.csv").unwrap();
        assert!(!ticks.is_empty());
        assert!(ticks.iter().all(|t| t.price > 0.0));
        // Timestamps strictly non-decreasing.
        for w in ticks.windows(2) {
            assert!(w[0].timestamp_secs <= w[1].timestamp_secs);
        }
    }

    #[test]
    fn end_to_end_on_csv_fixture() {
        let ticks = load_ticks_csv("tests/fixtures/backtest_sample.csv").unwrap();
        let strategy = FixedWidth {
            half_width_frac: 0.08,
        };
        let report = run_backtest(&ticks, &strategy, base_cfg()).unwrap();
        assert!(report.total_fees > 0.0);
        assert!(report.max_drawdown <= 0.0);
        assert!((report.net_pnl - (report.total_fees + report.total_il_quote)).abs() < 1e-9);
    }
}
