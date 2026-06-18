//! Synthesise `pool_ticks` rows from GeckoTerminal OHLCV history.
//!
//! GeckoTerminal gives price + USD volume but not on-chain tick liquidity or
//! `fee_growth_global`. We reconstruct the fields the DB-replay backtest needs:
//!
//! - `sqrt_price` / `tick_current` ← candle close price (raw domain).
//! - `liquidity` ← a constant pool-liquidity estimate (CLI `--pool-liquidity`).
//! - `fee_growth_global_b` ← a running accumulator derived from per-candle volume.
//!
//! ## Fee-growth synthesis (the core)
//! Pool fees over a candle = `volume_usd * fee_rate`, charged on the quote
//! (token-B / USDC) side. `fee_growth_global` is *per unit of pool liquidity*
//! in Q64.64, so the per-candle increment is:
//!
//! ```text
//! Δfg_b = volume_usd * fee_rate * 10^decimals_b * 2^64 / pool_liquidity
//! ```
//!
//! Replaying this through `db_replay` (which computes
//! `fees = Δfg/2^64 * position_liquidity / 10^decimals_b`) yields exactly
//! `volume_usd * fee_rate * (position_L / pool_L)` — the position's real share
//! of pool fees. The round-trip is covered by a golden test below.
//!
//! Approximations (accepted for research, same spirit as T-03-09): constant pool
//! liquidity, all fees attributed to the quote side, and the first candle's
//! volume is the delta baseline (not counted) — negligible over a multi-month run.

use crate::data::geckoterminal::OhlcvCandle;
use crate::math::sqrt_price::price_to_sqrt_q64;
use crate::storage::writer::PoolTick;

const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0; // 2^64

/// Pool-level parameters needed to turn OHLCV into `pool_ticks` rows.
#[derive(Debug, Clone)]
pub struct PoolSynthParams {
    pub pool_address: String,
    /// Pool fee rate in basis points (e.g. 4.0 = 0.04%).
    pub fee_rate_bps: f64,
    /// Constant pool liquidity estimate (Q64.64 `L`). Must be > 0 to accrue fees.
    pub pool_liquidity: u128,
    pub decimals_a: u8,
    pub decimals_b: u8,
    pub tick_spacing: i32,
}

/// Turn a chronological OHLCV series into synthetic `pool_ticks` rows.
///
/// Candles must be sorted ascending by timestamp (as `geckoterminal::fetch_range`
/// returns them). `fee_growth_global_b` accumulates across candles so that
/// consecutive-row deltas reproduce each candle's fee share in `db_replay`.
pub fn synthesize_ticks(candles: &[OhlcvCandle], p: &PoolSynthParams) -> Vec<PoolTick> {
    let ui_factor = 10f64.powi(p.decimals_a as i32 - p.decimals_b as i32);
    let scale_b = 10f64.powi(p.decimals_b as i32);
    let fee_rate = p.fee_rate_bps / 10_000.0;
    let pool_l = p.pool_liquidity as f64;

    let mut fee_growth_b: u128 = 0;
    let mut rows = Vec::with_capacity(candles.len());

    for c in candles {
        // Raw-domain price (token-B base units per token-A base unit).
        let raw_price = c.close / ui_factor;
        let sqrt_price = price_to_sqrt_q64(raw_price);
        let tick_current = super::price_to_tick(raw_price, p.tick_spacing);

        // Per-candle fee-growth increment on the quote side.
        if pool_l > 0.0 && c.volume_usd > 0.0 {
            let dfg = c.volume_usd * fee_rate * scale_b / pool_l * TWO_POW_64;
            if dfg.is_finite() && dfg > 0.0 {
                let dfg = if dfg >= u128::MAX as f64 {
                    u128::MAX
                } else {
                    dfg as u128
                };
                fee_growth_b = fee_growth_b.saturating_add(dfg);
            }
        }

        rows.push(PoolTick {
            pool_address: p.pool_address.clone(),
            slot: c.timestamp, // unix seconds — unique & monotonic per candle
            tick_current,
            sqrt_price,
            liquidity: p.pool_liquidity,
            fee_growth_global_a: 0,
            fee_growth_global_b: fee_growth_b,
            observed_at: chrono::DateTime::from_timestamp(c.timestamp, 0).unwrap_or_default(),
        });
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(ts: i64, close: f64, volume_usd: f64) -> OhlcvCandle {
        OhlcvCandle {
            timestamp: ts,
            open: close,
            high: close,
            low: close,
            close,
            volume_usd,
        }
    }

    fn params() -> PoolSynthParams {
        PoolSynthParams {
            pool_address: "POOL".to_string(),
            fee_rate_bps: 10.0,            // 0.1%
            pool_liquidity: 1_000_000_000, // 1e9
            decimals_a: 9,
            decimals_b: 6,
            tick_spacing: 64,
        }
    }

    // close=100 UI, raw=0.1; vol 0 / 1e6 / 2e6 USD.
    fn candles() -> Vec<OhlcvCandle> {
        vec![
            candle(1_000_000, 100.0, 0.0),
            candle(1_086_400, 100.0, 1_000_000.0),
            candle(1_172_800, 100.0, 2_000_000.0),
        ]
    }

    #[test]
    fn fee_growth_accumulates_in_q64_64() {
        // Δfg = vol * 0.001 * 1e6 * 2^64 / 1e9.
        // candle1: 1e6*0.001*1e6/1e9 = 1.0 → Δ = 2^64.
        // candle2: 2.0 → Δ = 2^65. Running: [0, 2^64, 3*2^64].
        let rows = synthesize_ticks(&candles(), &params());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].fee_growth_global_b, 0);
        assert_eq!(rows[1].fee_growth_global_b, 1u128 << 64);
        assert_eq!(rows[2].fee_growth_global_b, 3u128 << 64);
        // Quote-side only; base side stays zero.
        assert!(rows.iter().all(|r| r.fee_growth_global_a == 0));
        // Constant liquidity + constant price.
        assert!(rows.iter().all(|r| r.liquidity == 1_000_000_000));
        assert_eq!(rows[0].tick_current, rows[2].tick_current);
    }

    #[test]
    fn sqrt_price_matches_close() {
        let rows = synthesize_ticks(&candles(), &params());
        let raw = crate::math::sqrt_price::sqrt_q64_to_price(rows[0].sqrt_price);
        assert!((raw - 0.1).abs() / 0.1 < 1e-9, "raw price {raw}");
    }

    #[test]
    fn zero_liquidity_accrues_no_fees() {
        let mut p = params();
        p.pool_liquidity = 0;
        let rows = synthesize_ticks(&candles(), &p);
        assert!(rows.iter().all(|r| r.fee_growth_global_b == 0));
    }

    /// The whole point: synthesised ticks replayed through `db_replay` must
    /// reproduce the position's real share of pool fees.
    /// (v1+v2) * fee_rate * (pos_L/pool_L) = 3e6 * 0.001 * (1e8/1e9) = $300.
    #[test]
    fn round_trips_through_db_replay() {
        use crate::backtest::db_replay::{DbBacktestInput, run_db_backtest};
        use crate::storage::tick_reader::PoolTickRow;
        use crate::strategy::RebalanceConfig;

        let rows = synthesize_ticks(&candles(), &params());
        let ticks: Vec<PoolTickRow> = rows
            .iter()
            .map(|t| PoolTickRow {
                time: t.observed_at,
                pool_address: t.pool_address.clone(),
                slot: t.slot,
                tick_current: t.tick_current,
                sqrt_price: t.sqrt_price,
                liquidity: t.liquidity,
                fee_growth_global_a: t.fee_growth_global_a,
                fee_growth_global_b: t.fee_growth_global_b,
            })
            .collect();

        let input = DbBacktestInput {
            initial_value_usd: 10_000.0,
            entry_price: 100.0,
            price_lower: 1.0, // very wide range → stays in range, IL ≈ 0
            price_upper: 10_000.0,
            fee_rate_bps: 10.0,
            tick_spacing: 64,
            position_liquidity: 100_000_000, // 1e8 → share 0.1
            decimals_a: 9,
            decimals_b: 6,
            rebalance_cfg: RebalanceConfig {
                rebalance_out_of_range: false,
                near_edge_ticks: 0,
                min_net_pnl_usd: 0.0,
            },
            range_factor_lower: 0.95,
            range_factor_upper: 1.05,
        };

        let result = run_db_backtest(input, &ticks).unwrap();
        assert!(
            (result.total_fees_usd - 300.0).abs() < 0.01,
            "expected ~$300 fees, got {}",
            result.total_fees_usd
        );
    }
}
