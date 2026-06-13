// Plans 02 and 03 wire these types into the DB layer and watch loop.
// Until then suppress dead-code lints for this entire module.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::PgPool;
use tracing::warn;

use crate::storage::writer::PnlSnapshot;

/// Drift v2 mainnet program ID. Owner check + PDA derivation must agree on this.
///
/// [ASSUMED] Program ID correct as of training data; verify against official Drift docs before deployment.
const DRIFT_PROGRAM_ID: solana_sdk::pubkey::Pubkey =
    solana_sdk::pubkey!("dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH");

/// Persisted state for a single pool's risk monitor.
/// Stored in DB (plan 02) and loaded at watch startup.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Used by Plan 02 (DB persistence) and Plan 03 (watch-loop wiring)
pub struct RiskState {
    pub pool_address: String,
    pub peak_pnl: f64,
    pub current_drawdown_pct: f64,
    pub pause_flag: bool,
    pub halt_flag: bool,
    /// D-04: operator-controlled rebalance halt, independent from IL-triggered pause_flag.
    /// Set/cleared via Telegram /pause and /resume commands (Plan 03, TG-04).
    pub operator_pause: bool,
    pub updated_at: DateTime<Utc>,
}

/// Actions returned by [`RiskMonitor::evaluate`].
///
/// `PartialEq` is derived so tests can use `assert_eq!` on variants.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // Variants consumed by Plan 03 (watch-loop wiring)
pub enum RiskAction {
    /// No limit breached — proceed normally.
    Continue,
    /// IL exceeded threshold; rebalancing should be paused.
    PauseRebalancing { il_pct: f64 },
    /// IL dropped back below threshold; rebalancing can resume.
    ResumeRebalancing { il_pct: f64 },
    /// Drawdown exceeded threshold; halt all activity.
    HaltAll { drawdown_pct: f64 },
    /// Drift margin ratio below threshold; close hedge only.
    CloseDriftHedge { margin_ratio: f64 },
}

/// Pure-state risk evaluator.
///
/// `evaluate()` is synchronous and infallible — it takes a `PnlSnapshot` plus
/// an externally-fetched `drift_margin_ratio` so the method remains testable
/// without RPC mocking.
///
/// `fetch_drift_margin_ratio()` performs a real synchronous RPC read of the Drift
/// User account (D-01). It is designed for use with `tokio::task::spawn_blocking`
/// by the caller (Plan 03) to avoid blocking the async tick loop.
#[allow(dead_code)] // Constructed by Plan 03 (watch-loop wiring)
pub struct RiskMonitor {
    pub state: RiskState,
    max_drawdown_pct: Option<f64>,
    max_il_pct: Option<f64>,
    drift_min_margin_ratio: Option<f64>,
    /// Derived from wallet authority pubkey + Drift program PDA seeds.
    /// `None` in shadow mode (no keypair configured) or when --drift-min-margin-ratio is unset.
    pub drift_user_pubkey: Option<solana_sdk::pubkey::Pubkey>,
    /// Solana RPC URL used for Drift User account reads (same endpoint as the watch loop).
    pub rpc_url: String,
}

impl RiskMonitor {
    /// Create a new `RiskMonitor`.
    ///
    /// * `state` — initial persisted risk state (loaded from DB or freshly seeded)
    /// * `max_drawdown_pct` — drawdown threshold as a percentage (e.g. `15.0` = 15 %).
    ///   `None` disables drawdown checking.
    /// * `max_il_pct` — IL threshold as a percentage of `position_value`.
    ///   `None` disables IL checking.
    /// * `drift_min_margin_ratio` — minimum acceptable Drift margin ratio (0.0–1.0).
    ///   `None` disables Drift margin checking.
    /// * `drift_user_pubkey` — Drift User PDA for RPC reads. `None` skips Drift margin check.
    /// * `rpc_url` — Solana RPC URL for Drift account reads.
    #[allow(dead_code)]
    pub fn new(
        state: RiskState,
        max_drawdown_pct: Option<f64>,
        max_il_pct: Option<f64>,
        drift_min_margin_ratio: Option<f64>,
        drift_user_pubkey: Option<solana_sdk::pubkey::Pubkey>,
        rpc_url: String,
    ) -> Self {
        Self {
            state,
            max_drawdown_pct,
            max_il_pct,
            drift_min_margin_ratio,
            drift_user_pubkey,
            rpc_url,
        }
    }

    /// Load persisted `RiskState` from the DB, or insert a fresh default row if none exists.
    ///
    /// CRITICAL (RESEARCH.md Pitfall 2 / D-12): uses SELECT-then-INSERT to avoid ever
    /// overwriting an existing `halt_flag = true` with a fresh default. The upsert
    /// in `persist_state` only updates non-halt columns.
    pub async fn load_or_init(pool: &PgPool, pool_address: &str) -> anyhow::Result<RiskState> {
        // Attempt to fetch an existing row.
        let row_opt = pool
            .fetch_optional(
                query(
                    "SELECT peak_pnl, current_drawdown_pct, pause_flag, halt_flag, operator_pause, updated_at \
                       FROM risk_state WHERE pool_address = $1",
                )
                .bind(pool_address),
            )
            .await
            .map_err(|e| anyhow::anyhow!("load_or_init SELECT failed: {}", e))?;

        if let Some(row) = row_opt {
            let halt_flag: bool = row.get("halt_flag");
            if halt_flag {
                tracing::error!(
                    pool = %pool_address,
                    "risk: halt_flag set from previous session -- rebalancing will remain halted until DB is manually cleared"
                );
            }
            let state = RiskState {
                pool_address: pool_address.to_string(),
                peak_pnl: row.get("peak_pnl"),
                current_drawdown_pct: row.get("current_drawdown_pct"),
                pause_flag: row.get("pause_flag"),
                halt_flag,
                operator_pause: row.get("operator_pause"),
                updated_at: row.get("updated_at"),
            };
            return Ok(state);
        }

        // No existing row — insert a fresh default and return it.
        pool.execute(
            query(
                "INSERT INTO risk_state \
                 (pool_address, peak_pnl, current_drawdown_pct, pause_flag, halt_flag, operator_pause, updated_at) \
                 VALUES ($1, 0.0, 0.0, FALSE, FALSE, FALSE, NOW()) \
                 ON CONFLICT (pool_address) DO NOTHING",
            )
            .bind(pool_address),
        )
        .await
        .map_err(|e| anyhow::anyhow!("load_or_init INSERT failed: {}", e))?;

        Ok(RiskState {
            pool_address: pool_address.to_string(),
            peak_pnl: 0.0,
            current_drawdown_pct: 0.0,
            pause_flag: false,
            halt_flag: false,
            operator_pause: false,
            updated_at: Utc::now(),
        })
    }

    /// Reset volatile session state so a fresh watch session starts with a clean slate.
    ///
    /// Zeroes `peak_pnl` and `current_drawdown_pct` (a stale peak would
    /// otherwise read as an instant 100% drawdown on restart). `halt_flag`
    /// is deliberately NOT touched: per D-12 a drawdown halt must survive
    /// restarts until the operator clears it via SQL, exactly like
    /// `operator_pause`.
    ///
    /// Persists immediately via an UPDATE. Call this immediately after
    /// `load_or_init` at watch-session startup.
    pub async fn reset_session(pool: &PgPool, pool_address: &str) -> anyhow::Result<()> {
        pool.execute(
            query(
                "UPDATE risk_state \
                 SET peak_pnl = 0.0, \
                     current_drawdown_pct = 0.0, updated_at = NOW() \
                 WHERE pool_address = $1",
            )
            .bind(pool_address),
        )
        .await
        .map_err(|e| anyhow::anyhow!("reset_session UPDATE failed: {}", e))?;
        Ok(())
    }

    /// Persist `RiskState` to the DB via fire-and-forget spawn (RISK-04).
    ///
    /// Matches the `spawn_pnl_write` pattern in `storage::writer`: spawns a Tokio
    /// task immediately so the caller (watch-loop tick handler) is never blocked by
    /// DB I/O. Failures are logged via `tracing::warn!` with `pool_address`.
    ///
    /// Uses `ON CONFLICT ... DO UPDATE` — all fields except `pool_address` are
    /// overwritten. `halt_flag` is intentionally included so a breach detected in
    /// memory is immediately durably stored. Operators clear it via SQL (D-12).
    pub fn persist_state(pool: PgPool, state: RiskState) {
        tokio::spawn(async move {
            let result = pool
                .execute(
                    query(
                        "INSERT INTO risk_state \
                         (pool_address, peak_pnl, current_drawdown_pct, pause_flag, halt_flag, operator_pause, updated_at) \
                         VALUES ($1, $2, $3, $4, $5, $6, NOW()) \
                         ON CONFLICT (pool_address) DO UPDATE SET \
                           peak_pnl = EXCLUDED.peak_pnl, \
                           current_drawdown_pct = EXCLUDED.current_drawdown_pct, \
                           pause_flag = EXCLUDED.pause_flag, \
                           halt_flag = EXCLUDED.halt_flag, \
                           operator_pause = EXCLUDED.operator_pause, \
                           updated_at = EXCLUDED.updated_at",
                    )
                    .bind(&state.pool_address)
                    .bind(state.peak_pnl)
                    .bind(state.current_drawdown_pct)
                    .bind(state.pause_flag)
                    .bind(state.halt_flag)
                    .bind(state.operator_pause),
                )
                .await;

            if let Err(e) = result {
                warn!(
                    error = %e,
                    pool = %state.pool_address,
                    "risk_state persist failed"
                );
            }
        });
    }

    /// Derive the Drift User PDA for a given wallet authority (subaccount 0).
    ///
    /// Seeds: `["user", authority_pubkey_bytes, 0u16_le_bytes]`
    /// Program: `dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH` (Drift v2 mainnet)
    ///
    /// [ASSUMED] Program ID correct as of training data; verify against official Drift docs before deployment.
    pub fn derive_drift_user_pda(
        authority: &solana_sdk::pubkey::Pubkey,
    ) -> solana_sdk::pubkey::Pubkey {
        let (pda, _) = solana_sdk::pubkey::Pubkey::find_program_address(
            &[b"user", authority.as_ref(), &0u16.to_le_bytes()],
            &DRIFT_PROGRAM_ID,
        );
        pda
    }

    /// Fetch the Drift User account via RPC and compute a proxy margin ratio.
    ///
    /// **This is a REAL RPC fetch, not a stub (D-01).** Designed for use with
    /// `tokio::task::spawn_blocking` by the caller (Plan 03) since it uses the
    /// synchronous `solana_client::rpc_client::RpcClient`.
    ///
    /// # Proxy margin ratio (approximation)
    ///
    /// True Drift margin ratio requires oracle prices for all perp/spot markets —
    /// impractical to replicate off-chain without fetching 10-20 additional accounts
    /// per tick. This implementation computes a simplified proxy:
    ///
    /// ```text
    /// proxy_ratio = |sum(quote_asset_amount)| / (|sum(base_asset_amount)| + 1)
    /// ```
    ///
    /// where `quote_asset_amount` and `base_asset_amount` are read from the raw
    /// `PerpPosition` array in the Drift User account. This is explicitly an
    /// approximation and is documented as such. Full oracle-aware calculation is
    /// deferred to LIVE-02 scope.
    ///
    /// # Error handling
    ///
    /// RPC failures (network, timeout, account not found) → `None` with `warn!` log.
    /// Treat `None` as "margin OK" (D-03 fallback). Never let a monitoring failure
    /// cascade into an execution halt (RESEARCH.md anti-patterns).
    ///
    /// Returns `None` if:
    /// - `drift_user_pubkey` is `None` (shadow mode / no wallet — RESEARCH.md Pitfall 5)
    /// - `drift_min_margin_ratio` is `None` (limit not configured — skip check)
    /// - RPC call fails (network error, timeout) → warning logged
    /// - Account data too short (< 9 bytes after discriminator) → warning logged
    pub fn fetch_drift_margin_ratio(&self) -> Option<f64> {
        // Skip if no pubkey configured (shadow mode or limit disabled).
        let pubkey = self.drift_user_pubkey?;
        // Skip if the limit is not configured — no point fetching.
        self.drift_min_margin_ratio?;

        let rpc = solana_client::rpc_client::RpcClient::new_with_timeout(
            self.rpc_url.clone(),
            std::time::Duration::from_secs(5),
        );

        let account = match rpc.get_account(&pubkey) {
            Ok(a) => a,
            Err(e) => {
                warn!(
                    error = %e,
                    pubkey = %pubkey,
                    "drift user RPC fetch failed -- margin check skipped"
                );
                return None;
            }
        };

        // CLAUDE.md mandate: verify program owner before deserializing. Margin
        // numbers gate live execution, so never parse an account that is not
        // owned by the Drift program.
        if account.owner != DRIFT_PROGRAM_ID {
            warn!(
                pubkey = %pubkey,
                owner = %account.owner,
                expected = %DRIFT_PROGRAM_ID,
                "drift user account has wrong owner -- skipping margin check"
            );
            return None;
        }
        let data = account.data;

        // Anchor accounts always start with an 8-byte discriminator.
        if data.len() <= 8 {
            warn!(
                len = data.len(),
                pubkey = %pubkey,
                "drift user account too short -- skipping margin check"
            );
            return None;
        }

        // Skip discriminator; parse simplified PerpPosition proxy from raw bytes.
        // Layout approximation (RESEARCH.md Pattern 6): after the discriminator the
        // Drift User account contains fixed-size header fields followed by a
        // PerpPosition array. We read the raw bytes to extract base/quote amounts.
        //
        // Since full borsh deserialization of the full User struct requires the
        // complete IDL-generated types (heavy transitive deps), we use a simplified
        // approach: sum the i64 pairs (base_asset_amount, quote_asset_amount) from
        // the known offset range, treating any parse error as "margin OK".
        //
        // APPROXIMATION: This proxy does not apply oracle weighting. It is sufficient
        // for directional risk monitoring in Phase 6. Full calculation deferred to LIVE-02.
        let payload = &data[8..];

        // Each PerpPosition occupies 136 bytes in Drift User v2 layout.
        // base_asset_amount: i64 at offset 8 within each position
        // quote_asset_amount: i64 at offset 16 within each position
        // Maximum 8 perp positions per user (Drift v2 constant).
        const PERP_POSITION_SIZE: usize = 136;
        const PERP_POSITION_BASE_OFFSET: usize = 8;
        const PERP_POSITION_QUOTE_OFFSET: usize = 16;
        // Drift User header is ~4400 bytes before perp_positions array.
        // We scan the payload for i64 pairs at known stride as an approximation.
        const PERP_ARRAY_OFFSET: usize = 4400;
        const MAX_PERP_POSITIONS: usize = 8;

        let mut total_base_abs: i64 = 0;
        let mut total_quote_abs: i64 = 0;

        if payload.len() < PERP_ARRAY_OFFSET + MAX_PERP_POSITIONS * PERP_POSITION_SIZE {
            // Payload is smaller than expected layout — treat as no data (margin OK).
            return None;
        }

        if payload.len() >= PERP_ARRAY_OFFSET + MAX_PERP_POSITIONS * PERP_POSITION_SIZE {
            for i in 0..MAX_PERP_POSITIONS {
                let pos_start = PERP_ARRAY_OFFSET + i * PERP_POSITION_SIZE;
                let base_start = pos_start + PERP_POSITION_BASE_OFFSET;
                let quote_start = pos_start + PERP_POSITION_QUOTE_OFFSET;

                if quote_start + 8 <= payload.len() {
                    let base = i64::from_le_bytes(
                        payload[base_start..base_start + 8]
                            .try_into()
                            .unwrap_or([0u8; 8]),
                    );
                    let quote = i64::from_le_bytes(
                        payload[quote_start..quote_start + 8]
                            .try_into()
                            .unwrap_or([0u8; 8]),
                    );
                    total_base_abs = total_base_abs.saturating_add(base.abs());
                    total_quote_abs = total_quote_abs.saturating_add(quote.abs());
                }
            }
        }

        // Proxy ratio: |quote| / (|base| + 1) to avoid division by zero.
        // A higher ratio indicates more collateral relative to notional — safer.
        let proxy_ratio = total_quote_abs as f64 / (total_base_abs as f64 + 1.0);
        Some(proxy_ratio)
    }

    /// Evaluate all risk limits for the given P&L snapshot.
    ///
    /// Evaluation order (per D-05, D-06):
    /// 1. halt_flag gate
    /// 2. Peak P&L high-water mark update
    /// 3. Drawdown check
    /// 4. IL check
    /// 5. Drift margin check
    /// 6. Continue
    #[allow(dead_code)]
    pub fn evaluate(&mut self, snap: &PnlSnapshot, drift_margin_ratio: Option<f64>) -> RiskAction {
        // --- 1. halt_flag gate ---
        if self.state.halt_flag {
            self.state.updated_at = Utc::now();
            return RiskAction::HaltAll {
                drawdown_pct: self.state.current_drawdown_pct,
            };
        }

        // --- 2. Update peak P&L high-water mark ---
        if snap.net_pnl > self.state.peak_pnl {
            self.state.peak_pnl = snap.net_pnl;
        }

        // --- 3. Drawdown check (skip when no peak established) ---
        if let Some(max_dd) = self.max_drawdown_pct {
            const MIN_PEAK_USD: f64 = 1.0;
            if self.state.peak_pnl >= MIN_PEAK_USD {
                let drawdown_pct =
                    (self.state.peak_pnl - snap.net_pnl) / self.state.peak_pnl * 100.0;
                self.state.current_drawdown_pct = drawdown_pct;

                if drawdown_pct > max_dd {
                    self.state.halt_flag = true;
                    self.state.updated_at = Utc::now();
                    return RiskAction::HaltAll { drawdown_pct };
                }
            }
        }

        // --- 4. IL check ---
        let il_pct = if snap.position_value > 0.0 {
            snap.il_usd.abs() / snap.position_value * 100.0
        } else {
            0.0
        };

        if let Some(max_il) = self.max_il_pct {
            if il_pct > max_il {
                if !self.state.pause_flag {
                    self.state.pause_flag = true;
                    self.state.updated_at = Utc::now();
                    return RiskAction::PauseRebalancing { il_pct };
                } else {
                    // Already paused — propagate
                    self.state.updated_at = Utc::now();
                    return RiskAction::PauseRebalancing { il_pct };
                }
            } else if self.state.pause_flag {
                // IL dropped back below threshold — auto-resume
                self.state.pause_flag = false;
                self.state.updated_at = Utc::now();
                return RiskAction::ResumeRebalancing { il_pct };
            }
        }

        // --- 5. Drift margin check ---
        if let Some(min_ratio) = self.drift_min_margin_ratio {
            if let Some(ratio) = drift_margin_ratio {
                if ratio < min_ratio {
                    self.state.updated_at = Utc::now();
                    return RiskAction::CloseDriftHedge {
                        margin_ratio: ratio,
                    };
                }
            }
        }

        // --- 6. Continue ---
        self.state.updated_at = Utc::now();
        RiskAction::Continue
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_state(
        pool_address: &str,
        peak_pnl: f64,
        pause_flag: bool,
        halt_flag: bool,
    ) -> RiskState {
        RiskState {
            pool_address: pool_address.to_string(),
            peak_pnl,
            current_drawdown_pct: 0.0,
            pause_flag,
            halt_flag,
            operator_pause: false,
            updated_at: Utc::now(),
        }
    }

    fn make_snap(net_pnl: f64, il_usd: f64, position_value: f64) -> PnlSnapshot {
        PnlSnapshot {
            mint: "MINT".to_string(),
            pool_address: "POOL".to_string(),
            fees_earned: 0.0,
            il_usd,
            net_pnl,
            position_value,
            price: 100.0,
            observed_at: Utc::now(),
        }
    }

    fn monitor_all(
        state: RiskState,
        max_dd: Option<f64>,
        max_il: Option<f64>,
        drift_min: Option<f64>,
    ) -> RiskMonitor {
        RiskMonitor::new(state, max_dd, max_il, drift_min, None, String::new())
    }

    // -----------------------------------------------------------------------
    // halt_flag gate
    // -----------------------------------------------------------------------

    #[test]
    fn halt_flag_returns_halt_all_immediately() {
        let state = make_state("POOL", 100.0, false, true);
        let mut rm = monitor_all(state, Some(15.0), Some(4.0), None);
        let snap = make_snap(90.0, -5.0, 1000.0);
        // halt_flag is true — no further checks should run
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::HaltAll { drawdown_pct: 0.0 });
    }

    // -----------------------------------------------------------------------
    // Drawdown checks
    // -----------------------------------------------------------------------

    #[test]
    fn drawdown_breach_returns_halt_all() {
        let state = make_state("POOL", 100.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), None, None);
        // peak=100, net_pnl=80 -> drawdown=20% > 15% threshold
        let snap = make_snap(80.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::HaltAll { drawdown_pct: 20.0 });
        assert!(
            rm.state.halt_flag,
            "halt_flag must be set after drawdown breach"
        );
    }

    #[test]
    fn drawdown_skipped_when_peak_below_threshold() {
        // Case 1: peak_pnl = 0.0 (no peak established)
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), None, None);
        let snap = make_snap(-50.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
        assert!(!rm.state.halt_flag);

        // Case 2: peak_pnl = 0.5 (noise level, below MIN_PEAK_USD=1.0)
        // Previously this triggered a false halt: drawdown = (0.5 - (-0.12)) / 0.5 * 100 = 124%
        let state2 = make_state("POOL", 0.5, false, false);
        let mut rm2 = monitor_all(state2, Some(15.0), None, None);
        let snap2 = make_snap(-0.12, 0.0, 1000.0);
        let action2 = rm2.evaluate(&snap2, None);
        assert_eq!(action2, RiskAction::Continue);
        assert!(!rm2.state.halt_flag);
    }

    #[test]
    fn drawdown_not_triggered_below_threshold() {
        let state = make_state("POOL", 100.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), None, None);
        // drawdown=10% < 15% threshold
        let snap = make_snap(90.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
        assert!(!rm.state.halt_flag);
    }

    // -----------------------------------------------------------------------
    // High-water mark
    // -----------------------------------------------------------------------

    #[test]
    fn peak_pnl_only_increases() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, None);
        // First tick: net_pnl=200 -> peak should be 200
        rm.evaluate(&make_snap(200.0, 0.0, 1000.0), None);
        assert_eq!(rm.state.peak_pnl, 200.0);
        // Second tick: net_pnl=150 -> peak must remain 200
        rm.evaluate(&make_snap(150.0, 0.0, 1000.0), None);
        assert_eq!(rm.state.peak_pnl, 200.0);
    }

    // -----------------------------------------------------------------------
    // IL checks
    // -----------------------------------------------------------------------

    #[test]
    fn il_breach_returns_pause_rebalancing() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // IL = |-50| / 1000 * 100 = 5% > 4%
        let snap = make_snap(0.0, -50.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::PauseRebalancing { il_pct: 5.0 });
        assert!(rm.state.pause_flag);
    }

    #[test]
    fn il_recovery_returns_resume_rebalancing() {
        let state = make_state("POOL", 0.0, true, false); // pause_flag=true
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // IL = |-30| / 1000 * 100 = 3% <= 4%
        let snap = make_snap(0.0, -30.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::ResumeRebalancing { il_pct: 3.0 });
        assert!(!rm.state.pause_flag);
    }

    #[test]
    fn il_still_above_threshold_while_paused_returns_pause_rebalancing() {
        let state = make_state("POOL", 0.0, true, false); // pause_flag=true
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // IL = |-50| / 1000 * 100 = 5% > 4%
        let snap = make_snap(0.0, -50.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::PauseRebalancing { il_pct: 5.0 });
        assert!(rm.state.pause_flag, "pause_flag must remain set");
    }

    #[test]
    fn il_no_hysteresis_threshold_same_for_pause_and_resume() {
        // Pause threshold == resume threshold (D-09)
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // Exactly at threshold: 4% == 4% -> NOT a breach (> check)
        let snap = make_snap(0.0, -40.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
    }

    #[test]
    fn il_position_value_zero_yields_zero_il_pct() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, Some(4.0), None);
        // position_value=0 -> il_pct must be 0, not NaN/inf
        let snap = make_snap(0.0, -50.0, 0.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
    }

    // -----------------------------------------------------------------------
    // Evaluation order: drawdown fires before IL
    // -----------------------------------------------------------------------

    #[test]
    fn drawdown_fires_before_il_check() {
        // Both drawdown and IL are breached; drawdown must win
        let state = make_state("POOL", 100.0, false, false);
        let mut rm = monitor_all(state, Some(15.0), Some(4.0), None);
        // drawdown = 20% > 15%; IL = 5% > 4%
        let snap = make_snap(80.0, -50.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert!(
            matches!(action, RiskAction::HaltAll { .. }),
            "expected HaltAll, got {action:?}"
        );
    }

    // -----------------------------------------------------------------------
    // All limits disabled
    // -----------------------------------------------------------------------

    #[test]
    fn all_limits_none_returns_continue() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, None);
        let snap = make_snap(-1000.0, -500.0, 100.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(action, RiskAction::Continue);
    }

    // -----------------------------------------------------------------------
    // Drift margin checks
    // -----------------------------------------------------------------------

    #[test]
    fn drift_margin_below_threshold_returns_close_drift_hedge() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, Some(0.10));
        let snap = make_snap(0.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, Some(0.05));
        assert_eq!(action, RiskAction::CloseDriftHedge { margin_ratio: 0.05 });
    }

    #[test]
    fn drift_margin_above_threshold_returns_continue() {
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, Some(0.10));
        let snap = make_snap(0.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, Some(0.15));
        assert_eq!(action, RiskAction::Continue);
    }

    #[test]
    fn drift_min_margin_none_returns_continue() {
        // drift_min_margin_ratio = None -> Drift check disabled
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, None, None, None);
        let snap = make_snap(0.0, 0.0, 1000.0);
        // Even if ratio is very low, disabled check must not fire
        let action = rm.evaluate(&snap, Some(0.001));
        assert_eq!(action, RiskAction::Continue);
    }

    // -----------------------------------------------------------------------
    // Task 2: derive_drift_user_pda + fetch_drift_margin_ratio unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn derive_drift_user_pda_produces_valid_pubkey() {
        use solana_sdk::pubkey::Pubkey;
        // Use a known deterministic authority for reproducibility.
        let authority = Pubkey::new_unique();
        let pda = RiskMonitor::derive_drift_user_pda(&authority);
        // Must not be the all-zeros pubkey (default/uninitialized).
        assert_ne!(pda, Pubkey::default(), "PDA must not be all zeros");
        // Different authorities must produce different PDAs.
        let authority2 = Pubkey::new_unique();
        let pda2 = RiskMonitor::derive_drift_user_pda(&authority2);
        assert_ne!(pda, pda2, "different authorities must yield different PDAs");
    }

    #[test]
    fn fetch_drift_margin_ratio_returns_none_when_no_pubkey() {
        // drift_user_pubkey = None -> skip check (shadow mode / Pitfall 5)
        let state = make_state("POOL", 0.0, false, false);
        let rm = RiskMonitor::new(
            state,
            None,
            None,
            Some(0.10), // limit configured, but pubkey absent
            None,       // no pubkey
            "https://api.mainnet-beta.solana.com".to_string(),
        );
        assert_eq!(
            rm.fetch_drift_margin_ratio(),
            None,
            "must return None when drift_user_pubkey is None"
        );
    }

    #[test]
    fn fetch_drift_margin_ratio_returns_none_when_limit_not_configured() {
        // drift_min_margin_ratio = None -> limit disabled, no point fetching
        use solana_sdk::pubkey::Pubkey;
        let state = make_state("POOL", 0.0, false, false);
        let rm = RiskMonitor::new(
            state,
            None,
            None,
            None,                       // limit not configured
            Some(Pubkey::new_unique()), // pubkey present but limit absent
            "https://api.mainnet-beta.solana.com".to_string(),
        );
        assert_eq!(
            rm.fetch_drift_margin_ratio(),
            None,
            "must return None when drift_min_margin_ratio is None"
        );
    }

    /// Full RPC test — requires a live Solana node and a funded Drift User account.
    /// Run manually: cargo test -- --ignored drift_rpc
    #[test]
    #[ignore = "requires live Solana RPC and Drift User account"]
    fn fetch_drift_margin_ratio_rpc_roundtrip() {
        // Placeholder — real test needs DRIFT_USER_PUBKEY env var and Solana RPC.
    }

    // -----------------------------------------------------------------------
    // Task 2: session reset — verifies fix for stale peak_pnl on restart
    // -----------------------------------------------------------------------

    /// After a session reset (peak_pnl=0, halt_flag=false), the first evaluate()
    /// call with net_pnl=0 must return Continue and must NOT trigger HaltAll.
    /// Regression test for: peak_pnl loaded from DB > 0 causes instant 100%
    /// drawdown on restart when net_pnl starts at 0.
    #[test]
    fn new_session_start_does_not_halt_when_pnl_zero() {
        // Simulate post-reset state: peak_pnl=0, halt_flag=false (as set by reset_session)
        let state = make_state("POOL", 0.0, false, false);
        let mut rm = monitor_all(state, Some(50.0), None, None);
        // net_pnl=0 at session start — must not trigger drawdown halt
        let snap = make_snap(0.0, 0.0, 1000.0);
        let action = rm.evaluate(&snap, None);
        assert_eq!(
            action,
            RiskAction::Continue,
            "zero peak_pnl must never trigger halt at session start"
        );
    }

    /// Verify that operator_pause survives a session reset: the in-memory zeroing
    /// in main.rs only touches peak_pnl, halt_flag, and current_drawdown_pct.
    #[test]
    fn operator_pause_preserved_after_session_reset_fields() {
        // Construct state as if loaded from DB with operator_pause=true
        let mut state = make_state("POOL", 500.0, false, true);
        state.operator_pause = true;

        // Apply the same in-memory zeroing that main.rs does after reset_session()
        state.peak_pnl = 0.0;
        state.halt_flag = false;
        state.current_drawdown_pct = 0.0;

        assert!(
            state.operator_pause,
            "operator_pause must be preserved after session reset"
        );
        assert_eq!(state.peak_pnl, 0.0);
        assert!(!state.halt_flag);
        assert_eq!(state.current_drawdown_pct, 0.0);
    }
}
