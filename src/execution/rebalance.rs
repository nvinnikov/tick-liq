/// Dry-run rebalance plan: a description of the instruction sequence that
/// *would* be submitted to land a close -> collect -> open rebalance.
///
/// No Solana RPC calls, no transaction construction, no signing. This type
/// exists so the CLI can preview a rebalance before any real execution path
/// is wired up.
#[derive(Debug, Clone)]
pub struct RebalancePlan {
    pub position_mint: String,
    pub close_ix_count: usize,
    pub collect_ix_count: usize,
    pub open_ix_count: usize,
    pub estimated_cu: u64,
    pub new_tick_lower: i32,
    pub new_tick_upper: i32,
}

/// Build a centered-rebalance plan: the old range's width is preserved and
/// re-centered on `current_tick`, with both bounds aligned to `tick_spacing`
/// so the new range always contains the current price. Pure function — no I/O.
pub fn build_rebalance_plan(
    position_mint: &str,
    current_tick: i32,
    tick_lower: i32,
    tick_upper: i32,
    tick_spacing: i32,
) -> RebalancePlan {
    let spacing = tick_spacing.max(1);
    let width = tick_upper.saturating_sub(tick_lower).max(spacing);
    let half = width / 2;

    // Floor-align to tick_spacing (Euclidean so negative ticks align down).
    let align = |t: i32| -> i32 { t - t.rem_euclid(spacing) };

    let mut new_tick_lower = align(current_tick.saturating_sub(half));
    let mut new_tick_upper = new_tick_lower.saturating_add(width);

    // Guarantee the current tick is strictly inside [lower, upper].
    if new_tick_upper <= current_tick {
        new_tick_upper = align(current_tick).saturating_add(spacing);
    }
    if new_tick_lower > current_tick {
        new_tick_lower = align(current_tick);
    }

    RebalancePlan {
        position_mint: position_mint.to_string(),
        close_ix_count: 1,
        collect_ix_count: 1,
        open_ix_count: 1,
        estimated_cu: 200_000,
        new_tick_lower,
        new_tick_upper,
    }
}

/// Print the plan in the format required by F13.
pub fn print_dry_run(plan: &RebalancePlan) {
    println!("Rebalance Plan (DRY RUN -- no transaction sent)");
    println!("Position:     {}", plan.position_mint);
    println!("Steps:        1. close_position  2. collect_fees  3. open_position (new range)");
    println!(
        "Instructions: close={}  collect={}  open={}",
        plan.close_ix_count, plan.collect_ix_count, plan.open_ix_count
    );
    println!("Est. CU:      ~{}", plan.estimated_cu);
    println!(
        "New range:    [{}, {}]  (centered rebalance)",
        plan.new_tick_lower, plan.new_tick_upper
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centers_preserved_width_on_current_tick() {
        // Old range [100, 200] (width 100), price moved to tick 1000.
        let plan = build_rebalance_plan("MINT", 1000, 100, 200, 8);
        assert_eq!(plan.new_tick_upper - plan.new_tick_lower, 100);
        assert!(plan.new_tick_lower <= 1000 && 1000 <= plan.new_tick_upper);
        assert_eq!(plan.new_tick_lower.rem_euclid(8), 0);
        assert_eq!(plan.close_ix_count, 1);
        assert_eq!(plan.collect_ix_count, 1);
        assert_eq!(plan.open_ix_count, 1);
        assert_eq!(plan.estimated_cu, 200_000);
    }

    #[test]
    fn out_of_range_price_is_recaptured() {
        // The pre-fix behavior (widen old range by 10*spacing) left tick 5000
        // outside the proposed range for old range [0, 1000], spacing 64.
        let plan = build_rebalance_plan("MINT", 5000, 0, 1000, 64);
        assert!(
            plan.new_tick_lower <= 5000 && 5000 <= plan.new_tick_upper,
            "current tick must be inside the new range: [{}, {}]",
            plan.new_tick_lower,
            plan.new_tick_upper
        );
        assert_eq!(plan.new_tick_upper - plan.new_tick_lower, 1000);
    }

    #[test]
    fn handles_negative_ticks() {
        let plan = build_rebalance_plan("MINT", -300, -500, -100, 10);
        assert!(plan.new_tick_lower <= -300 && -300 <= plan.new_tick_upper);
        assert_eq!(plan.new_tick_upper - plan.new_tick_lower, 400);
        assert_eq!(plan.new_tick_lower.rem_euclid(10), 0);
    }

    #[test]
    fn saturates_on_extreme_ticks() {
        let plan = build_rebalance_plan("MINT", 0, i32::MIN + 5, i32::MAX - 5, 1000);
        assert!(plan.new_tick_lower <= 0 && 0 <= plan.new_tick_upper);
    }
}
