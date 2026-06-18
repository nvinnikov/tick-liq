pub mod cex_ws;
pub mod coinbase_ws;
pub mod ws;

/// A price source, used as the `source` label on price/feed metrics.
// Not yet wired into call-sites — upcoming metrics/Coinbase tasks will use it.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Binance,
    Coinbase,
    Orca,
}

impl Source {
    /// Stable lowercase label for metrics (must stay stable — dashboards key on it).
    // Not yet called from production paths — upcoming metrics tasks will use it.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Source::Binance => "binance",
            Source::Coinbase => "coinbase",
            Source::Orca => "orca",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_labels_are_stable() {
        assert_eq!(Source::Binance.label(), "binance");
        assert_eq!(Source::Coinbase.label(), "coinbase");
        assert_eq!(Source::Orca.label(), "orca");
    }
}
