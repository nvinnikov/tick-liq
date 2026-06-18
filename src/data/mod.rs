pub mod cex_ws;
pub mod coinbase_ws;
pub mod ws;

/// A price source, used as the `source` label on price/feed metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    Binance,
    Coinbase,
    Orca,
}

impl Source {
    /// Stable lowercase label for metrics (must stay stable — dashboards key on it).
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
