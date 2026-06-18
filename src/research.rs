//! Research sweep: backtest a matrix of (pool × range-width × rebalance) over
//! real GeckoTerminal history, fully in memory (no database), and emit one tidy
//! CSV row per run for offline analysis (`research/analyze.py`).
//!
//! Reuses the tested building blocks: `geckoterminal::fetch_range` (data),
//! `backfill::synthesize_ticks` (OHLCV → pool_ticks), `db_replay::run_db_backtest`
//! (P&L), and `math::metrics` (risk metrics). Nothing here touches Postgres.
//!
//! ## Comparable across pools
//! Every run deploys the same fixed `capital_usd`. Because fee%, IL% and net%
//! are invariant to the absolute position liquidity (both fees and capital scale
//! linearly with L), we scale `position_liquidity` so the position is worth
//! exactly `capital_usd` at entry — then $-figures and APYs are directly
//! comparable across pools and range widths.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::analytics::amounts::compute_token_amounts;
use crate::backtest::backfill::{PoolSynthParams, synthesize_ticks};
use crate::backtest::db_replay::{DbBacktestInput, run_db_backtest};
use crate::backtest::price_to_tick;
use crate::data::geckoterminal::fetch_range;
use crate::math::metrics::{RiskMetrics, annualized_volatility, daily_returns};
use crate::math::sqrt_price::price_to_sqrt_q64;
use crate::storage::tick_reader::PoolTickRow;
use crate::storage::writer::PoolTick;
use crate::strategy::RebalanceConfig;

/// Disables near-edge rebalancing: `tick - bound <= NEAR_EDGE_OFF` can never be
/// true (a tick range spans at most ~887k), so only the explicit out-of-range
/// path can fire. NOTE `near_edge_ticks = 0` does NOT disable it — it rebalances
/// AT the boundary — so the `rebalance = false` arm must use this to be a true
/// hold, and the `rebalance = true` arm isolates the out-of-range trigger.
const NEAR_EDGE_OFF: i32 = -1_000_000;

// ── Config (TOML) ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub settings: Settings,
    pub pools: Vec<PoolCfg>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    /// Inclusive start date, YYYY-MM-DD (UTC).
    pub from: String,
    /// Exclusive end date, YYYY-MM-DD (UTC).
    pub to: String,
    /// GeckoTerminal timeframe: day | hour | minute.
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    /// Capital deployed per run, USD.
    pub capital_usd: f64,
    /// Half-width fractions to sweep, e.g. [0.05, 0.10, 0.20] = ±5/10/20%.
    pub range_widths: Vec<f64>,
    /// Rebalance variants to run. Empty ⇒ `[false]`.
    #[serde(default)]
    pub rebalance: Vec<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoolCfg {
    pub label: String,
    pub address: String,
    pub fee_bps: f64,
    pub decimals_a: u8,
    pub decimals_b: u8,
    pub tick_spacing: i32,
    /// On-chain pool liquidity L, as a decimal string (u128 exceeds TOML i64).
    pub pool_liquidity: String,
}

fn default_timeframe() -> String {
    "day".to_string()
}

// ── Result row ──────────────────────────────────────────────────────────────

/// One backtest run, flattened for CSV.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub label: String,
    pub address: String,
    pub range_width: f64,
    pub rebalance: bool,
    pub capital_usd: f64,
    /// Modelled position liquidity as a fraction of pool liquidity. When this is
    /// not small (≳0.25) the constant-L share approximation overstates fees —
    /// a thin-pool / narrow-range honesty flag for downstream analysis.
    pub pool_share: f64,
    pub days: usize,
    pub total_rebalances: u32,
    pub total_fees_usd: f64,
    pub il_usd: f64,
    pub net_pnl_usd: f64,
    pub net_pct: f64,
    pub fee_apy_pct: f64,
    pub days_in_range_pct: f64,
    pub realized_vol: f64,
    pub sharpe: Option<f64>,
    pub sortino: Option<f64>,
    pub max_drawdown: f64,
}

/// CSV header matching `RunResult::to_csv_row`.
pub fn csv_header() -> String {
    "label,address,range_width,rebalance,capital_usd,pool_share,days,total_rebalances,total_fees_usd,il_usd,\
     net_pnl_usd,net_pct,fee_apy_pct,days_in_range_pct,realized_vol,sharpe,sortino,max_drawdown"
        .to_string()
}

impl RunResult {
    pub fn to_csv_row(&self) -> String {
        let opt = |o: Option<f64>| o.map_or_else(|| "".to_string(), |v| format!("{v:.6}"));
        format!(
            "{},{},{:.4},{},{:.2},{:.6},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.2},{:.6},{},{},{:.6}",
            self.label,
            self.address,
            self.range_width,
            self.rebalance,
            self.capital_usd,
            self.pool_share,
            self.days,
            self.total_rebalances,
            self.total_fees_usd,
            self.il_usd,
            self.net_pnl_usd,
            self.net_pct,
            self.fee_apy_pct,
            self.days_in_range_pct,
            self.realized_vol,
            opt(self.sharpe),
            opt(self.sortino),
            self.max_drawdown,
        )
    }
}

// ── Run ─────────────────────────────────────────────────────────────────────

fn parse_day(s: &str) -> Result<i64> {
    let d: chrono::NaiveDate = s
        .parse()
        .with_context(|| format!("invalid date '{s}' (expected YYYY-MM-DD)"))?;
    Ok(d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp())
}

fn to_rows(ticks: &[PoolTick]) -> Vec<PoolTickRow> {
    ticks
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
        .collect()
}

/// Run the full sweep. Fetches each pool's history once, then replays every
/// (range-width × rebalance) cell over it. Pools that fail to fetch / are too
/// short are logged and skipped rather than aborting the whole sweep.
pub async fn run(cfg: &Config, client: &reqwest::Client) -> Result<Vec<RunResult>> {
    let from_ts = parse_day(&cfg.settings.from)?;
    let to_ts = parse_day(&cfg.settings.to)?;
    if to_ts <= from_ts {
        bail!("settings.to must be after settings.from");
    }
    let rebalance_variants: Vec<bool> = if cfg.settings.rebalance.is_empty() {
        vec![false]
    } else {
        cfg.settings.rebalance.clone()
    };

    let mut out = Vec::new();
    for pool in &cfg.pools {
        let pool_l: u128 = pool
            .pool_liquidity
            .parse()
            .with_context(|| format!("pool '{}' liquidity is not a u128", pool.label))?;
        if pool_l == 0 {
            tracing::warn!(pool = pool.label, "pool_liquidity is 0; skipping");
            continue;
        }

        let candles = match fetch_range(
            client,
            &pool.address,
            &cfg.settings.timeframe,
            from_ts,
            to_ts,
        )
        .await
        {
            Ok(c) if c.len() >= 3 => c,
            Ok(c) => {
                tracing::warn!(
                    pool = pool.label,
                    got = c.len(),
                    "too few candles; skipping"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(pool = pool.label, error = %e, "fetch failed; skipping");
                continue;
            }
        };

        // Synthesise the pool's tick stream once (independent of position range).
        let synth = PoolSynthParams {
            pool_address: pool.address.clone(),
            fee_rate_bps: pool.fee_bps,
            pool_liquidity: pool_l,
            decimals_a: pool.decimals_a,
            decimals_b: pool.decimals_b,
            tick_spacing: pool.tick_spacing,
        };
        let rows = to_rows(&synthesize_ticks(&candles, &synth));

        let entry = candles[0].close;
        let ui_factor = 10f64.powi(pool.decimals_a as i32 - pool.decimals_b as i32);
        let scale_a = 10f64.powi(pool.decimals_a as i32);
        let scale_b = 10f64.powi(pool.decimals_b as i32);

        // Realized (annualised) volatility of the underlying price. metrics'
        // annualisation assumes daily (×√365); rescale for the chosen timeframe.
        let periods_per_year: f64 = match cfg.settings.timeframe.as_str() {
            "hour" => 365.0 * 24.0,
            "minute" => 365.0 * 24.0 * 60.0,
            _ => 365.0,
        };
        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        let realized_vol =
            annualized_volatility(&daily_returns(&closes)) * (periods_per_year / 365.0).sqrt();

        for &width in &cfg.settings.range_widths {
            let lower = entry * (1.0 - width);
            let upper = entry * (1.0 + width);

            // Scale position liquidity so the position is worth `capital_usd` at
            // entry (keeps $-figures comparable across pools / widths).
            let sqrt_entry = price_to_sqrt_q64(entry / ui_factor);
            let tick_lower = price_to_tick(lower / ui_factor, pool.tick_spacing);
            let tick_upper = price_to_tick(upper / ui_factor, pool.tick_spacing);
            let amt = compute_token_amounts(pool_l, sqrt_entry, tick_lower, tick_upper)?;
            let value_at_pool_l =
                amt.amount_a as f64 / scale_a * entry + amt.amount_b as f64 / scale_b;
            if value_at_pool_l <= 0.0 {
                continue;
            }
            let pos_l = (pool_l as f64 * cfg.settings.capital_usd / value_at_pool_l) as u128;

            for &rebalance in &rebalance_variants {
                let input = DbBacktestInput {
                    initial_value_usd: cfg.settings.capital_usd,
                    entry_price: entry,
                    price_lower: lower,
                    price_upper: upper,
                    fee_rate_bps: pool.fee_bps,
                    tick_spacing: pool.tick_spacing,
                    position_liquidity: pos_l,
                    decimals_a: pool.decimals_a,
                    decimals_b: pool.decimals_b,
                    rebalance_cfg: RebalanceConfig {
                        rebalance_out_of_range: rebalance,
                        near_edge_ticks: NEAR_EDGE_OFF,
                        min_net_pnl_usd: 0.0,
                    },
                    range_factor_lower: 1.0 - width,
                    range_factor_upper: 1.0 + width,
                };

                let res = run_db_backtest(input, &rows)
                    .with_context(|| format!("backtest {} w={width}", pool.label))?;
                let daily_net: Vec<f64> = res.days.iter().map(|d| d.net_pnl_usd).collect();
                let m = RiskMetrics::from_backtest(cfg.settings.capital_usd, &daily_net);
                let n = res.days.len().max(1);

                out.push(RunResult {
                    label: pool.label.clone(),
                    address: pool.address.clone(),
                    range_width: width,
                    rebalance,
                    capital_usd: cfg.settings.capital_usd,
                    pool_share: pos_l as f64 / pool_l as f64,
                    days: res.days.len(),
                    total_rebalances: res.total_rebalances,
                    total_fees_usd: res.total_fees_usd,
                    il_usd: res.total_il_usd,
                    net_pnl_usd: res.net_pnl_usd,
                    net_pct: res.net_pnl_usd / cfg.settings.capital_usd * 100.0,
                    fee_apy_pct: res.fee_apy_pct,
                    days_in_range_pct: res.days_in_range as f64 / n as f64 * 100.0,
                    realized_vol,
                    sharpe: m.sharpe,
                    sortino: m.sortino,
                    max_drawdown: m.max_drawdown,
                });
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"
        [settings]
        from = "2026-01-01"
        to = "2026-02-01"
        capital_usd = 10000.0
        range_widths = [0.05, 0.10]
        rebalance = [false, true]

        [[pools]]
        label = "SOL/USDC"
        address = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE"
        fee_bps = 4.0
        decimals_a = 9
        decimals_b = 6
        tick_spacing = 4
        pool_liquidity = "123456789012345"
    "#;

    #[test]
    fn parses_config() {
        let cfg: Config = toml::from_str(TOML).unwrap();
        assert_eq!(cfg.settings.capital_usd, 10000.0);
        assert_eq!(cfg.settings.range_widths, vec![0.05, 0.10]);
        assert_eq!(cfg.pools.len(), 1);
        assert_eq!(
            cfg.pools[0].pool_liquidity.parse::<u128>().unwrap(),
            123456789012345
        );
    }

    #[test]
    fn timeframe_defaults_to_day() {
        let cfg: Config = toml::from_str(TOML).unwrap();
        assert_eq!(cfg.settings.timeframe, "day");
    }

    #[test]
    fn csv_row_matches_header_arity() {
        let r = RunResult {
            label: "X".into(),
            address: "A".into(),
            range_width: 0.1,
            rebalance: false,
            capital_usd: 10_000.0,
            pool_share: 0.12,
            days: 30,
            total_rebalances: 4,
            total_fees_usd: 120.0,
            il_usd: -40.0,
            net_pnl_usd: 80.0,
            net_pct: 0.8,
            fee_apy_pct: 14.6,
            days_in_range_pct: 73.3,
            realized_vol: 0.65,
            sharpe: Some(1.2),
            sortino: None,
            max_drawdown: -0.05,
        };
        assert_eq!(
            csv_header().split(',').count(),
            r.to_csv_row().split(',').count()
        );
    }
}
