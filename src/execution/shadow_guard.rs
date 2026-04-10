use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowGuard {
    Shadow,
    Live,
}

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ShadowGuardError {
    #[error("transaction submission blocked: shadow mode is active")]
    Blocked,
}

impl ShadowGuard {
    pub fn shadow() -> Self {
        Self::Shadow
    }
    pub fn live() -> Self {
        Self::Live
    }
    #[allow(dead_code)]
    pub fn is_shadow(&self) -> bool {
        matches!(self, Self::Shadow)
    }

    /// Called at the single point where a rebalance transaction would be submitted.
    /// Phase 5 replaces the submit call with direct ShadowGuard match; kept for
    /// backward-compat and test coverage.
    #[allow(dead_code)]
    pub fn submit<T: std::fmt::Debug>(&self, tx: &T) -> Result<(), ShadowGuardError> {
        match self {
            Self::Shadow => {
                tracing::warn!(?tx, "ShadowGuard: submission blocked (shadow mode)");
                Err(ShadowGuardError::Blocked)
            }
            Self::Live => {
                tracing::info!(?tx, "ShadowGuard: submission allowed (live mode)");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_blocks_submit() {
        let g = ShadowGuard::shadow();
        assert!(g.is_shadow());
        let result = g.submit(&"dummy_tx");
        assert!(matches!(result, Err(ShadowGuardError::Blocked)));
    }

    #[test]
    fn live_allows_submit() {
        let g = ShadowGuard::live();
        assert!(!g.is_shadow());
        let result = g.submit(&"dummy_tx");
        assert!(result.is_ok());
    }

    #[test]
    fn is_shadow_matches_constructor() {
        assert!(ShadowGuard::shadow().is_shadow());
        assert!(!ShadowGuard::live().is_shadow());
    }
}
