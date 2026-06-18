//! Research-grade risk metrics for a backtest equity curve.
//!
//! Pure `f64` math, **zero** external deps — same ethos as the rest of `math/`.
//! Operates on a daily equity curve (USD), so it works identically for the GBM
//! simulator and the DB-replay backtest (both emit `BacktestResult`).
//!
//! Annualisation uses 365 trading days (crypto trades every calendar day).

/// Trading periods per year for annualisation (crypto: every calendar day).
pub const PERIODS_PER_YEAR: f64 = 365.0;

/// Aggregate risk metrics derived from a daily equity curve.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskMetrics {
    /// Annualised volatility of daily returns (fraction, e.g. 0.42 = 42%).
    pub annual_volatility: f64,
    /// Annualised Sharpe ratio. `None` when undefined (zero variance / <2 returns).
    pub sharpe: Option<f64>,
    /// Annualised Sortino ratio. `None` when there is no downside deviation.
    pub sortino: Option<f64>,
    /// Maximum drawdown as a fraction of the running peak (≤ 0, e.g. -0.25).
    pub max_drawdown: f64,
    /// Calmar ratio: annualised return / |max drawdown|. `None` when no drawdown.
    pub calmar: Option<f64>,
    /// Total return over the whole period (fraction).
    pub total_return: f64,
}

impl RiskMetrics {
    /// Build metrics from the position's starting value and the per-day
    /// **cumulative** net P&L series (one entry per simulated day), as produced
    /// by `BacktestResult.days[].net_pnl_usd`.
    pub fn from_backtest(initial_value_usd: f64, daily_cumulative_net_pnl: &[f64]) -> Self {
        let equity = equity_curve(initial_value_usd, daily_cumulative_net_pnl);
        Self::from_equity(&equity)
    }

    /// Build metrics directly from a daily equity curve (first point = open value).
    pub fn from_equity(equity: &[f64]) -> Self {
        let returns = daily_returns(equity);
        let total_return = match (equity.first(), equity.last()) {
            (Some(&start), Some(&end)) if start != 0.0 => (end - start) / start,
            _ => 0.0,
        };
        let max_drawdown = max_drawdown(equity);
        let annual_return = annualized_return(equity);
        RiskMetrics {
            annual_volatility: annualized_volatility(&returns),
            sharpe: annualized_sharpe(&returns),
            sortino: annualized_sortino(&returns),
            max_drawdown,
            calmar: calmar(annual_return, max_drawdown),
            total_return,
        }
    }
}

/// Prepend the open value, then chain `initial + cumulative_net_pnl` for each day.
pub fn equity_curve(initial_value_usd: f64, daily_cumulative_net_pnl: &[f64]) -> Vec<f64> {
    let mut curve = Vec::with_capacity(daily_cumulative_net_pnl.len() + 1);
    curve.push(initial_value_usd);
    for &net in daily_cumulative_net_pnl {
        curve.push(initial_value_usd + net);
    }
    curve
}

/// Simple daily returns `(eᵢ − eᵢ₋₁) / eᵢ₋₁`. Skips steps where the prior value is 0.
pub fn daily_returns(equity: &[f64]) -> Vec<f64> {
    equity
        .windows(2)
        .filter_map(|w| {
            let (prev, cur) = (w[0], w[1]);
            if prev == 0.0 {
                None
            } else {
                Some((cur - prev) / prev)
            }
        })
        .collect()
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Sample standard deviation (Bessel-corrected, n−1). `0.0` for <2 points.
fn sample_std(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let m = mean(xs);
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() as f64 - 1.0);
    var.sqrt()
}

/// Annualised volatility of daily returns.
pub fn annualized_volatility(returns: &[f64]) -> f64 {
    sample_std(returns) * PERIODS_PER_YEAR.sqrt()
}

/// Annualised Sharpe ratio (risk-free rate assumed 0). `None` if undefined.
pub fn annualized_sharpe(returns: &[f64]) -> Option<f64> {
    let sd = sample_std(returns);
    if sd == 0.0 {
        return None;
    }
    Some(mean(returns) / sd * PERIODS_PER_YEAR.sqrt())
}

/// Annualised Sortino ratio — like Sharpe but penalising only downside (returns < 0).
/// Downside deviation = √(Σ min(0, r)² / N). `None` when there is no downside.
pub fn annualized_sortino(returns: &[f64]) -> Option<f64> {
    if returns.is_empty() {
        return None;
    }
    let downside_sq: f64 = returns.iter().map(|r| r.min(0.0).powi(2)).sum();
    if downside_sq == 0.0 {
        return None;
    }
    let downside_dev = (downside_sq / returns.len() as f64).sqrt();
    Some(mean(returns) / downside_dev * PERIODS_PER_YEAR.sqrt())
}

/// Maximum drawdown of an equity curve as a fraction of the running peak (≤ 0).
pub fn max_drawdown(equity: &[f64]) -> f64 {
    let mut peak = f64::NEG_INFINITY;
    let mut max_dd = 0.0;
    for &v in equity {
        if v > peak {
            peak = v;
        }
        if peak > 0.0 {
            let dd = (v - peak) / peak;
            if dd < max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

/// Compound annualised return from the equity curve (CAGR over the simulated span).
pub fn annualized_return(equity: &[f64]) -> f64 {
    let n_periods = equity.len().saturating_sub(1);
    match (equity.first(), equity.last()) {
        (Some(&start), Some(&end)) if start > 0.0 && end > 0.0 && n_periods > 0 => {
            (end / start).powf(PERIODS_PER_YEAR / n_periods as f64) - 1.0
        }
        _ => 0.0,
    }
}

/// Calmar ratio: annualised return / |max drawdown|. `None` when there is no drawdown.
pub fn calmar(annual_return: f64, max_drawdown: f64) -> Option<f64> {
    if max_drawdown == 0.0 {
        None
    } else {
        Some(annual_return / max_drawdown.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-3;

    #[test]
    fn equity_curve_prepends_open_value() {
        let curve = equity_curve(1000.0, &[10.0, -5.0, 20.0]);
        assert_eq!(curve, vec![1000.0, 1010.0, 995.0, 1020.0]);
    }

    #[test]
    fn sharpe_golden() {
        // returns [0.02, -0.01, 0.03, 0.00]: mean 0.01, sample std 0.0182574,
        // daily 0.547723, annualised ×√365 = 10.4642.
        let returns = [0.02, -0.01, 0.03, 0.00];
        let s = annualized_sharpe(&returns).unwrap();
        assert!((s - 10.4642).abs() < TOL, "sharpe = {s}");
    }

    #[test]
    fn sortino_golden() {
        // same returns: downside dev = √((0.01²)/4) = 0.005, daily 2.0, ×√365 = 38.2099.
        let returns = [0.02, -0.01, 0.03, 0.00];
        let s = annualized_sortino(&returns).unwrap();
        assert!((s - 38.2099).abs() < TOL, "sortino = {s}");
    }

    #[test]
    fn max_drawdown_peak_to_trough() {
        // peak 120 → trough 90 → (90-120)/120 = -0.25.
        assert!((max_drawdown(&[100.0, 120.0, 90.0, 110.0]) - (-0.25)).abs() < 1e-12);
    }

    #[test]
    fn max_drawdown_monotonic_is_zero() {
        assert_eq!(max_drawdown(&[100.0, 110.0, 130.0, 200.0]), 0.0);
    }

    #[test]
    fn sharpe_zero_variance_is_none() {
        assert_eq!(annualized_sharpe(&[0.01, 0.01, 0.01]), None);
    }

    #[test]
    fn sharpe_too_few_points_is_none() {
        assert_eq!(annualized_sharpe(&[0.01]), None);
    }

    #[test]
    fn sortino_no_downside_is_none() {
        assert_eq!(annualized_sortino(&[0.01, 0.02, 0.0]), None);
    }

    #[test]
    fn symmetric_returns_have_zero_sharpe() {
        let s = annualized_sharpe(&[0.1, -0.1, 0.1, -0.1]).unwrap();
        assert!(s.abs() < TOL, "sharpe = {s}");
    }

    #[test]
    fn calmar_no_drawdown_is_none() {
        assert_eq!(calmar(0.5, 0.0), None);
    }

    #[test]
    fn calmar_basic() {
        // 50% annual return, -25% max DD → 2.0.
        assert!((calmar(0.5, -0.25).unwrap() - 2.0).abs() < 1e-12);
    }

    // ── Invariants ──────────────────────────────────────────────────────────

    #[test]
    fn max_drawdown_never_positive() {
        for curve in [
            vec![100.0, 50.0, 75.0, 25.0, 200.0],
            vec![10.0, 10.0, 10.0],
            vec![1.0, 2.0, 0.5, 3.0],
        ] {
            assert!(max_drawdown(&curve) <= 0.0);
        }
    }

    #[test]
    fn from_backtest_end_to_end() {
        // initial 10_000, cumulative net P&L per day.
        let m = RiskMetrics::from_backtest(10_000.0, &[100.0, 50.0, 250.0, 250.0]);
        assert!((m.total_return - 0.025).abs() < 1e-9); // 250/10000
        assert!(m.max_drawdown < 0.0); // dipped from 10_100 to 10_050
        assert!(m.sharpe.is_some());
    }
}
