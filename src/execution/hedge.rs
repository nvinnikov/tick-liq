/// Dry-run Drift perp hedge plan for an LP position.
///
/// An in-range CLMM LP is naturally short volatility (negative delta),
/// so the offsetting perp is a long. This module computes the required
/// notional size but does NOT build or send any Drift CPI — that wiring
/// is deferred to a later task.
#[derive(Debug, Clone)]
pub struct HedgePlan {
    pub position_mint: String,
    pub delta: f64,
    pub perp_size_usd: f64,
    pub perp_side: &'static str,
    pub description: String,
}

/// Given LP position delta (negative = short vol, typical for in-range LP),
/// compute the Drift perp size needed to neutralize it.
///
/// `delta < 0` → go long perp to offset; `delta > 0` → go short perp.
/// `perp_size_usd = |delta| * price`.
pub fn compute_hedge_size(delta: f64, price: f64) -> HedgePlan {
    let perp_size_usd = delta.abs() * price;
    let perp_side = if delta < 0.0 { "long" } else { "short" };
    HedgePlan {
        position_mint: String::new(),
        delta,
        perp_size_usd,
        perp_side,
        description: "Drift perp hedge (DRY RUN -- no ix sent)".to_string(),
    }
}

/// Print the plan in the F14 format.
pub fn print_hedge_dry_run(plan: &HedgePlan) {
    let offset_note = if plan.delta < 0.0 {
        "offsetting negative delta"
    } else if plan.delta > 0.0 {
        "offsetting positive delta"
    } else {
        "delta is zero"
    };
    println!("Hedge Plan (DRY RUN -- no instruction sent)");
    println!("Kind:         {}", plan.description);
    println!("Position:    {}", plan.position_mint);
    println!("Delta:       {:.4}", plan.delta);
    println!("Perp size:   ${:.2}", plan.perp_size_usd);
    println!("Side:        {}  ({})", plan.perp_side, offset_note);
    println!("Note:        Drift CPI not yet wired -- this is a size estimate only");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_delta_goes_long() {
        let plan = compute_hedge_size(-0.0123, 11585.37);
        assert_eq!(plan.perp_side, "long");
        assert!((plan.perp_size_usd - 0.0123 * 11585.37).abs() < 1e-9);
    }

    #[test]
    fn positive_delta_goes_short() {
        let plan = compute_hedge_size(0.5, 100.0);
        assert_eq!(plan.perp_side, "short");
        assert!((plan.perp_size_usd - 50.0).abs() < 1e-9);
    }

    #[test]
    fn zero_delta_zero_size() {
        let plan = compute_hedge_size(0.0, 1000.0);
        assert_eq!(plan.perp_size_usd, 0.0);
        assert_eq!(plan.perp_side, "short");
    }
}
