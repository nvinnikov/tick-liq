use crate::analytics::amounts::TokenAmounts;
use crate::analytics::depth::LiquidityLevel;
use crate::analytics::greeks::Greeks;
use crate::analytics::pnl::PnlResult;

/// Render an ASCII histogram of a liquidity distribution. The bar for the
/// bucket containing `current_price` is marked with `*`.
pub fn print_depth_histogram(levels: &[LiquidityLevel], current_price: f64) {
    if levels.is_empty() {
        println!("  (no tick array data available)");
        return;
    }
    let max_l = levels.iter().map(|l| l.liquidity).max().unwrap_or(0);
    if max_l == 0 {
        println!("  (all buckets empty)");
        return;
    }
    const WIDTH: usize = 40;

    // Find the bucket closest to current_price.
    let current_idx = levels
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            (a.price - current_price)
                .abs()
                .partial_cmp(&(b.price - current_price).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    for (i, level) in levels.iter().enumerate() {
        let bar_len = ((level.liquidity as f64 / max_l as f64) * WIDTH as f64).round() as usize;
        let bar = "█".repeat(bar_len);
        let marker = if i == current_idx { "*" } else { " " };
        println!(
            "  {} ${:>10.4}  {:<width$}  {:.2}M",
            marker,
            level.price,
            bar,
            level.liquidity as f64 / 1e6,
            width = WIDTH
        );
    }
}

// Uses owned Strings to avoid lifetime complexity (no &'a str).
pub struct PositionSummary {
    pub pool_address: String,
    pub fee_rate_bps: f64,
    pub price_lower: f64,
    pub price_upper: f64,
    pub price_current: f64,
    pub in_range: bool,
    pub range_pct: f64, // 0–100, position within the range
    pub amounts: TokenAmounts,
    pub decimals_a: u8,
    pub decimals_b: u8,
    pub symbol_a: String,
    pub symbol_b: String,
    pub pnl: PnlResult,
    pub greeks: Greeks,
}

pub fn print_position(s: &PositionSummary) {
    let label = format!(
        "Position: {}...  (Orca {:.2} bps)",
        &s.pool_address[..8],
        s.fee_rate_bps
    );
    let sep = "─".repeat(label.len());

    println!("{}", label);
    println!("{}", sep);

    let status = if s.in_range {
        format!("IN RANGE  ({:.0}%)", s.range_pct)
    } else {
        "OUT OF RANGE".to_string()
    };

    println!("Range:      ${:.4} -- ${:.4}", s.price_lower, s.price_upper);
    println!("Current:    ${:.4}  {}", s.price_current, status);
    println!();

    let a = s.amounts.amount_a as f64 / 10f64.powi(s.decimals_a as i32);
    let b = s.amounts.amount_b as f64 / 10f64.powi(s.decimals_b as i32);
    println!(
        "Amounts:    {:.6} {}  +  {:.2} {}",
        a, s.symbol_a, b, s.symbol_b
    );
    println!();

    println!("P&L:");
    println!(
        "  Fees:  {:+.2}  ({:+.2}%)",
        s.pnl.fees_usd,
        s.pnl.fees_pct()
    );
    println!("  IL:    {:+.2}  ({:+.2}%)", s.pnl.il_usd, s.pnl.il_pct());
    println!("  Net:   {:+.2}  ({:+.2}%)", s.pnl.net_usd, s.pnl.net_pct());
    println!();

    println!(
        "Delta: {:.4}   Gamma: {:.6}",
        s.greeks.delta, s.greeks.gamma
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::amounts::TokenAmounts;
    use crate::analytics::greeks::Greeks;
    use crate::analytics::pnl::PnlResult;

    #[test]
    fn test_print_position_does_not_panic() {
        let amounts = TokenAmounts {
            amount_a: 1_000_000_000,
            amount_b: 150_000_000,
        };
        let pnl = PnlResult {
            fees_usd: 10.0,
            il_usd: -3.0,
            net_usd: 7.0,
            initial_value_usd: 1000.0,
        };
        let greeks = Greeks {
            delta: -0.34,
            gamma: 0.02,
        };

        let s = PositionSummary {
            pool_address: "11111111111111111111111111111111".to_string(),
            fee_rate_bps: 30.0,
            price_lower: 100.0,
            price_upper: 200.0,
            price_current: 150.0,
            in_range: true,
            range_pct: 50.0,
            amounts,
            decimals_a: 9,
            decimals_b: 6,
            symbol_a: "SOL".to_string(),
            symbol_b: "USDC".to_string(),
            pnl,
            greeks,
        };

        print_position(&s); // should not panic
    }
}
