/// Configuration for the rebalance signal.
#[derive(Debug, Clone)]
pub struct RebalanceConfig {
    /// Rebalance when position is out of range.
    pub rebalance_out_of_range: bool,
    /// Rebalance when price is within this many ticks of the range boundary.
    pub near_edge_ticks: i32,
    /// Minimum net P&L required before rebalancing (avoid rebalancing at a loss).
    pub min_net_pnl_usd: f64,
}

impl Default for RebalanceConfig {
    fn default() -> Self {
        Self {
            rebalance_out_of_range: true,
            near_edge_ticks: 10,
            min_net_pnl_usd: 0.0,
        }
    }
}

/// Decision produced by `should_rebalance`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebalanceDecision {
    Hold { reason: String },
    Rebalance { reason: String },
}

/// Pure decision function: should this position be rebalanced?
pub fn should_rebalance(
    tick_current: i32,
    tick_lower: i32,
    tick_upper: i32,
    net_pnl_usd: f64,
    config: &RebalanceConfig,
) -> RebalanceDecision {
    if config.rebalance_out_of_range && (tick_current < tick_lower || tick_current > tick_upper) {
        return RebalanceDecision::Rebalance {
            reason: "out of range".to_string(),
        };
    }

    if net_pnl_usd < config.min_net_pnl_usd {
        return RebalanceDecision::Hold {
            reason: "P&L below threshold".to_string(),
        };
    }

    if tick_current - tick_lower <= config.near_edge_ticks {
        return RebalanceDecision::Rebalance {
            reason: "near lower edge".to_string(),
        };
    }

    if tick_upper - tick_current <= config.near_edge_ticks {
        return RebalanceDecision::Rebalance {
            reason: "near upper edge".to_string(),
        };
    }

    RebalanceDecision::Hold {
        reason: "position healthy".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RebalanceConfig {
        RebalanceConfig {
            rebalance_out_of_range: true,
            near_edge_ticks: 10,
            min_net_pnl_usd: 0.0,
        }
    }

    #[test]
    fn test_out_of_range_triggers_rebalance() {
        let d = should_rebalance(-100, 0, 1000, 100.0, &cfg());
        assert!(matches!(d, RebalanceDecision::Rebalance { .. }));
        if let RebalanceDecision::Rebalance { reason } = d {
            assert_eq!(reason, "out of range");
        }
    }

    #[test]
    fn test_above_range_triggers_rebalance() {
        let d = should_rebalance(2000, 0, 1000, 100.0, &cfg());
        assert!(matches!(d, RebalanceDecision::Rebalance { .. }));
    }

    #[test]
    fn test_near_lower_edge_triggers_rebalance() {
        let d = should_rebalance(5, 0, 1000, 100.0, &cfg());
        if let RebalanceDecision::Rebalance { reason } = d {
            assert_eq!(reason, "near lower edge");
        } else {
            panic!("expected Rebalance");
        }
    }

    #[test]
    fn test_near_upper_edge_triggers_rebalance() {
        let d = should_rebalance(995, 0, 1000, 100.0, &cfg());
        if let RebalanceDecision::Rebalance { reason } = d {
            assert_eq!(reason, "near upper edge");
        } else {
            panic!("expected Rebalance");
        }
    }

    #[test]
    fn test_healthy_position_holds() {
        let d = should_rebalance(500, 0, 1000, 100.0, &cfg());
        if let RebalanceDecision::Hold { reason } = d {
            assert_eq!(reason, "position healthy");
        } else {
            panic!("expected Hold");
        }
    }

    #[test]
    fn test_low_pnl_holds() {
        let c = RebalanceConfig {
            min_net_pnl_usd: 50.0,
            ..cfg()
        };
        let d = should_rebalance(500, 0, 1000, 10.0, &c);
        if let RebalanceDecision::Hold { reason } = d {
            assert_eq!(reason, "P&L below threshold");
        } else {
            panic!("expected Hold");
        }
    }
}
