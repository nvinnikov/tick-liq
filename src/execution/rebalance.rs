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

/// Build a centered-rebalance plan. The new range is the old range widened by
/// `tick_spacing * 10` on each side. Pure function — no I/O.
pub fn build_rebalance_plan(
    position_mint: &str,
    tick_lower: i32,
    tick_upper: i32,
    tick_spacing: i32,
) -> RebalancePlan {
    let widen = tick_spacing.saturating_mul(10);
    let new_tick_lower = tick_lower.saturating_sub(widen);
    let new_tick_upper = tick_upper.saturating_add(widen);

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
    fn widens_range_by_ten_tick_spacings() {
        let plan = build_rebalance_plan("MINT", 100, 200, 8);
        assert_eq!(plan.new_tick_lower, 20);
        assert_eq!(plan.new_tick_upper, 280);
        assert_eq!(plan.close_ix_count, 1);
        assert_eq!(plan.collect_ix_count, 1);
        assert_eq!(plan.open_ix_count, 1);
        assert_eq!(plan.estimated_cu, 200_000);
    }

    #[test]
    fn handles_negative_ticks() {
        let plan = build_rebalance_plan("MINT", -500, -100, 10);
        assert_eq!(plan.new_tick_lower, -600);
        assert_eq!(plan.new_tick_upper, 0);
    }

    #[test]
    fn saturates_on_extreme_ticks() {
        let plan = build_rebalance_plan("MINT", i32::MIN + 5, i32::MAX - 5, 1000);
        assert_eq!(plan.new_tick_lower, i32::MIN);
        assert_eq!(plan.new_tick_upper, i32::MAX);
    }
}
