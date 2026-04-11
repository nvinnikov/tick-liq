use anyhow::Result;
use clap::{Parser, Subcommand};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    Shadow,
    Live,
}

mod analytics;
mod backtest;
mod bot;
mod cache;
mod data;
mod display;
mod execution;
mod math;
mod protocols;
mod rpc;
mod storage;
mod strategy;

#[derive(Parser)]
#[command(name = "lp-inspect")]
#[command(about = "CLMM position inspector for Solana")]
struct Cli {
    #[arg(
        long,
        env = "SOLANA_RPC_URL",
        default_value = "https://api.devnet.solana.com"
    )]
    rpc_url: String,

    /// Postgres connection URL (e.g. postgres://user:pass@localhost/tickliq)
    #[arg(long, env = "DATABASE_URL")]
    db_url: Option<String>,

    /// Per-request RPC timeout in seconds (default 30). Each call is retried
    /// up to 3 times with exponential backoff before failing.
    #[arg(long, env = "RPC_TIMEOUT_SECS", default_value_t = 30u64)]
    rpc_timeout: u64,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Full P&L breakdown of a position
    Position {
        /// Position NFT mint address
        mint: String,
        /// Protocol: orca or raydium
        #[arg(long, default_value = "orca")]
        protocol: String,
        /// Entry price (token A denominated in token B) when position was opened.
        /// Used to compute impermanent loss. If omitted, IL will show 0.
        #[arg(long)]
        entry_price: Option<f64>,
    },
    /// Watch a position in real-time
    Watch {
        /// Position NFT mint address
        mint: String,
        /// Run in shadow mode: decisions logged, no transactions submitted (DEFAULT)
        #[arg(long, conflicts_with = "live")]
        shadow: bool,
        /// Run in live mode: submit real transactions (requires shadow gate passed)
        #[arg(long, conflicts_with = "shadow")]
        live: bool,
        /// Maximum cumulative P&L drawdown as percentage (e.g. 10.0 = 10%).
        /// When exceeded, LP position is closed and rebalancing halts permanently.
        #[arg(long)]
        max_drawdown: Option<f64>,
        /// Maximum instantaneous IL as percentage of position value (e.g. 5.0 = 5%).
        /// When exceeded, rebalancing is paused until IL recovers.
        #[arg(long)]
        max_il: Option<f64>,
        /// Minimum Drift margin ratio as percentage (e.g. 20.0 = 20%).
        /// When below this, Drift hedge close is logged (CPI deferred to LIVE-02).
        #[arg(long)]
        drift_min_margin_ratio: Option<f64>,
        /// Enable Telegram bot for rebalance approvals and operator commands.
        /// Requires TELEGRAM_BOT_TOKEN env var.
        #[arg(long)]
        telegram: bool,
        /// Telegram approval timeout in seconds (default 300 = 5 min).
        /// Rebalance is skipped if /approve not received within this window.
        #[arg(long, default_value_t = 300u64)]
        approve_timeout_secs: u64,
    },
    /// Liquidity distribution around current price
    Depth {
        /// Pool address
        pool: String,
    },
    /// Price impact for a specific trade size (USD)
    Impact {
        /// Pool address
        pool: String,
        #[arg(long)]
        size: f64,
    },
    /// Strategy-layer operations
    Strategy {
        #[command(subcommand)]
        command: StrategyCommands,
    },
    /// Database operations
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Build (but do not send) a rebalance plan for a position
    Rebalance {
        /// Position NFT mint address
        mint: String,
        /// Preview only; sending is not yet supported
        #[arg(long)]
        dry_run: bool,
    },
    /// Build (but do not send) a Drift perp hedge plan for a position
    Hedge {
        /// Position NFT mint address
        mint: String,
        /// Preview only; sending is not yet supported
        #[arg(long)]
        dry_run: bool,
    },
    /// Simulate LP P&L over a synthetic price path (GBM) or replay real DB ticks.
    ///
    /// GBM mode (default): requires --entry-price, --price-lower, --price-upper.
    ///
    /// DB mode (--pool <ADDR>): reads pool_ticks from TimescaleDB for the given
    /// address and date range. Requires --db-url / DATABASE_URL to be set.
    Backtest {
        /// Entry price (token A in token B units)
        #[arg(long)]
        entry_price: f64,
        /// Lower bound of LP range
        #[arg(long)]
        price_lower: f64,
        /// Upper bound of LP range
        #[arg(long)]
        price_upper: f64,
        /// Pool fee rate in basis points (e.g. 5 = 0.05%)
        #[arg(long, default_value_t = 5.0)]
        fee_bps: f64,
        /// Initial position value in USD
        #[arg(long, default_value_t = 10_000.0)]
        capital: f64,
        /// Number of days to simulate (GBM mode only)
        #[arg(long, default_value_t = 30)]
        days: u32,
        /// Annualised volatility, e.g. 0.80 for 80% (GBM mode only)
        #[arg(long, default_value_t = 0.80)]
        volatility: f64,
        /// Estimated daily volume through the pool in USD (GBM mode only)
        #[arg(long, default_value_t = 1_000_000.0)]
        daily_volume: f64,
        /// Fraction of pool daily volume captured by this position (0.0–1.0) (GBM mode only)
        #[arg(long, default_value_t = 0.10)]
        position_volume_share: f64,
        /// Tick spacing of the pool (used for rebalance signal)
        #[arg(long, default_value_t = 64)]
        tick_spacing: i32,
        /// Auto-rebalance when out of range
        #[arg(long)]
        rebalance: bool,
        /// Random seed for reproducibility (GBM mode only)
        #[arg(long, default_value_t = 42)]
        seed: u64,
        // ── DB mode flags ────────────────────────────────────────────────────
        /// Pool address to replay from TimescaleDB (enables DB mode)
        #[arg(long)]
        pool: Option<String>,
        /// Start date for DB replay (YYYY-MM-DD, inclusive)
        #[arg(long)]
        from: Option<String>,
        /// End date for DB replay (YYYY-MM-DD, exclusive)
        #[arg(long)]
        to: Option<String>,
        /// Position liquidity units held (required in DB mode)
        #[arg(long, default_value_t = 0u64)]
        position_liquidity: u64,
        /// Ticks from range boundary that trigger near-edge rebalance (DB mode; 0 = off)
        #[arg(long, default_value_t = 0i32)]
        near_edge_ticks: i32,
        /// Lower range width factor on rebalance (DB mode; e.g. 0.95 → lower = price * 0.95)
        #[arg(long, default_value_t = 0.95f64)]
        range_lower_factor: f64,
        /// Upper range width factor on rebalance (DB mode; e.g. 1.05 → upper = price * 1.05)
        #[arg(long, default_value_t = 1.05f64)]
        range_upper_factor: f64,
    },
}

#[derive(Subcommand)]
enum StrategyCommands {
    /// Evaluate rebalance signal for a position
    Check {
        /// Position NFT mint address
        mint: String,
        /// Rebalance when price is within this many ticks of a range boundary
        #[arg(long, default_value_t = 10)]
        near_edge_ticks: i32,
        /// Minimum net P&L (USD) required before rebalancing
        #[arg(long, default_value_t = 0.0)]
        min_pnl: f64,
        /// Entry price for IL calculation (optional)
        #[arg(long)]
        entry_price: Option<f64>,
    },
}

#[derive(Subcommand)]
enum DbAction {
    /// Run schema migrations
    Migrate,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match &cli.command {
        Commands::Position {
            mint,
            protocol,
            entry_price,
        } => {
            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);

            match protocol.as_str() {
                "orca" => {
                    use orca_whirlpools_core::tick_index_to_sqrt_price;

                    let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
                    let mint_pubkey = Pubkey::from_str(mint)?;
                    let (position_pda, _) = Pubkey::find_program_address(
                        &[b"position", mint_pubkey.as_ref()],
                        &whirlpool_program,
                    );

                    let position_data =
                        rpc.fetch_account_checked(&position_pda.to_string(), &whirlpool_program)?;
                    let pos = protocols::orca::parse_position(&position_data)?;

                    let pool_data =
                        rpc.fetch_account_checked(&pos.whirlpool.to_string(), &whirlpool_program)?;
                    let pool = protocols::orca::parse_pool(&pool_data)?;

                    // Fetch real decimals and symbols from chain.
                    let decimals_a = rpc.fetch_mint_decimals(&pool._token_mint_a)?;
                    let decimals_b = rpc.fetch_mint_decimals(&pool._token_mint_b)?;
                    let symbol_a = rpc.fetch_token_symbol(&pool._token_mint_a);
                    let symbol_b = rpc.fetch_token_symbol(&pool._token_mint_b);

                    use analytics::greeks::sqrt_q64_to_price;
                    let price_current = sqrt_q64_to_price(pool.sqrt_price);
                    let price_lower =
                        sqrt_q64_to_price(tick_index_to_sqrt_price(pos.tick_lower_index));
                    let price_upper =
                        sqrt_q64_to_price(tick_index_to_sqrt_price(pos.tick_upper_index));

                    let in_range = pool.tick_current_index >= pos.tick_lower_index
                        && pool.tick_current_index <= pos.tick_upper_index;
                    let range_pct = if in_range && (price_upper - price_lower) > 0.0 {
                        (price_current - price_lower) / (price_upper - price_lower) * 100.0
                    } else {
                        0.0
                    };

                    let amounts = analytics::amounts::compute_token_amounts(
                        pos.liquidity,
                        pool.sqrt_price,
                        pos.tick_lower_index,
                        pos.tick_upper_index,
                    )?;

                    let greeks = analytics::greeks::compute_greeks(
                        pos.liquidity,
                        pool.sqrt_price,
                        pos.tick_lower_index,
                        pos.tick_upper_index,
                    );

                    let accrued_a = analytics::pnl::compute_accrued_fees(
                        pool.fee_growth_global_a,
                        pos.fee_growth_checkpoint_a,
                        pos.liquidity,
                    );
                    let accrued_b = analytics::pnl::compute_accrued_fees(
                        pool.fee_growth_global_b,
                        pos.fee_growth_checkpoint_b,
                        pos.liquidity,
                    );

                    let scale_a = 10f64.powi(decimals_a as i32);
                    let scale_b = 10f64.powi(decimals_b as i32);

                    let fees_usd = (pos.fee_owed_a + accrued_a) as f64 / scale_a * price_current
                        + (pos.fee_owed_b + accrued_b) as f64 / scale_b;

                    // Resolve entry price: CLI flag takes precedence, then cache, then 0.
                    let price_entry = if let Some(ep) = entry_price {
                        cache::save_entry_price(mint, *ep)?;
                        *ep
                    } else if let Some(ep) = cache::load_entry_price(mint) {
                        tracing::info!("Loaded cached entry price: ${:.4}", ep);
                        ep
                    } else {
                        0.0
                    };
                    let il_fraction = analytics::pnl::compute_il(
                        price_entry,
                        price_current,
                        price_lower,
                        price_upper,
                    );
                    let position_value = amounts.amount_a as f64 / scale_a * price_current
                        + amounts.amount_b as f64 / scale_b;
                    let il_usd = il_fraction * position_value;

                    let pnl = analytics::pnl::PnlResult {
                        fees_usd,
                        il_usd,
                        net_usd: fees_usd + il_usd,
                        initial_value_usd: position_value,
                    };

                    let summary = display::table::PositionSummary {
                        pool_address: pos.whirlpool.to_string(),
                        fee_rate_bps: pool.fee_rate as f64 / 100.0,
                        price_lower,
                        price_upper,
                        price_current,
                        in_range,
                        range_pct,
                        amounts,
                        decimals_a,
                        decimals_b,
                        symbol_a,
                        symbol_b,
                        pnl,
                        greeks,
                    };

                    display::table::print_position(&summary);
                }
                "raydium" => {
                    let raydium_program = protocols::raydium::raydium_clmm_program_pubkey();
                    let mint_pubkey = Pubkey::from_str(mint)?;
                    let (position_pda, _) = Pubkey::find_program_address(
                        &[b"position", mint_pubkey.as_ref()],
                        &raydium_program,
                    );

                    let position_data =
                        rpc.fetch_account_checked(&position_pda.to_string(), &raydium_program)?;
                    let pos = protocols::raydium::parse_position(&position_data)?;

                    let pool_data =
                        rpc.fetch_account_checked(&pos.pool_id.to_string(), &raydium_program)?;
                    let pool = protocols::raydium::parse_pool(&pool_data)?;

                    use orca_whirlpools_core::tick_index_to_sqrt_price;

                    let price_current = analytics::greeks::sqrt_q64_to_price(pool.sqrt_price_x64);
                    let price_lower = analytics::greeks::sqrt_q64_to_price(
                        tick_index_to_sqrt_price(pos.tick_lower_index),
                    );
                    let price_upper = analytics::greeks::sqrt_q64_to_price(
                        tick_index_to_sqrt_price(pos.tick_upper_index),
                    );

                    let in_range = pool.tick_current >= pos.tick_lower_index
                        && pool.tick_current <= pos.tick_upper_index;
                    let range_pct = if in_range && (price_upper - price_lower) > 0.0 {
                        (price_current - price_lower) / (price_upper - price_lower) * 100.0
                    } else {
                        0.0
                    };

                    let amounts = analytics::amounts::compute_token_amounts(
                        pos.liquidity,
                        pool.sqrt_price_x64,
                        pos.tick_lower_index,
                        pos.tick_upper_index,
                    )?;

                    let greeks = analytics::greeks::compute_greeks(
                        pos.liquidity,
                        pool.sqrt_price_x64,
                        pos.tick_lower_index,
                        pos.tick_upper_index,
                    );

                    // TODO: Raydium fee tracking not yet wired (fee_growth_global not in pool struct)
                    let fees_usd = 0.0_f64;

                    // Resolve entry price: CLI flag takes precedence, then cache, then 0.
                    let price_entry = if let Some(ep) = entry_price {
                        cache::save_entry_price(mint, *ep)?;
                        *ep
                    } else if let Some(ep) = cache::load_entry_price(mint) {
                        tracing::info!("Loaded cached entry price: ${:.4}", ep);
                        ep
                    } else {
                        0.0
                    };
                    let il_fraction = analytics::pnl::compute_il(
                        price_entry,
                        price_current,
                        price_lower,
                        price_upper,
                    );

                    // TODO: Raydium decimals should be fetched from mint metadata;
                    // hardcoded to 9 (SOL) / 6 (USDC) as a temporary approximation.
                    let decimals_a: u8 = 9;
                    let decimals_b: u8 = 6;
                    let scale_a = 10f64.powi(decimals_a as i32);
                    let scale_b = 10f64.powi(decimals_b as i32);

                    let position_value = amounts.amount_a as f64 / scale_a * price_current
                        + amounts.amount_b as f64 / scale_b;
                    let il_usd = il_fraction * position_value;

                    let pnl = analytics::pnl::PnlResult {
                        fees_usd,
                        il_usd,
                        net_usd: fees_usd + il_usd,
                        initial_value_usd: position_value,
                    };

                    let summary = display::table::PositionSummary {
                        pool_address: pos.pool_id.to_string(),
                        fee_rate_bps: 0.0, // TODO: wire Raydium fee rate
                        price_lower,
                        price_upper,
                        price_current,
                        in_range,
                        range_pct,
                        amounts,
                        decimals_a,
                        decimals_b,
                        symbol_a: "SOL".to_string(), // TODO: fetch from mint metadata
                        symbol_b: "USDC".to_string(), // TODO: fetch from mint metadata
                        pnl,
                        greeks,
                    };

                    display::table::print_position(&summary);
                }
                other => anyhow::bail!("Unknown protocol '{}'. Use 'orca' or 'raydium'.", other),
            }
        }
        Commands::Watch {
            mint,
            shadow: _,
            live,
            max_drawdown,
            max_il,
            drift_min_margin_ratio,
            telegram,
            approve_timeout_secs,
        } => {
            // Validate risk limit flags
            if let Some(dd) = max_drawdown {
                if *dd <= 0.0 || *dd > 100.0 {
                    anyhow::bail!("--max-drawdown must be between 0 and 100 (got {})", dd);
                }
            }
            if let Some(il) = max_il {
                if *il <= 0.0 || *il > 100.0 {
                    anyhow::bail!("--max-il must be between 0 and 100 (got {})", il);
                }
            }
            if let Some(mr) = drift_min_margin_ratio {
                if *mr <= 0.0 {
                    anyhow::bail!("--drift-min-margin-ratio must be positive (got {})", mr);
                }
            }

            let max_drawdown_val: Option<f64> = *max_drawdown;
            let max_il_val: Option<f64> = *max_il;
            let drift_min_margin_ratio_val: Option<f64> = *drift_min_margin_ratio;

            let run_mode = if *live {
                RunMode::Live
            } else {
                RunMode::Shadow
            };
            tracing::info!(mode = ?run_mode, "watch starting");
            let guard = match run_mode {
                RunMode::Shadow => execution::ShadowGuard::shadow(),
                RunMode::Live => execution::ShadowGuard::live(),
            };
            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let mint_pubkey = Pubkey::from_str(mint)?;
            let (position_pda, _) = Pubkey::find_program_address(
                &[b"position", mint_pubkey.as_ref()],
                &whirlpool_program,
            );

            let position_data =
                rpc.fetch_account_checked(&position_pda.to_string(), &whirlpool_program)?;
            let pos = protocols::orca::parse_position(&position_data)?;
            let pool_addr = pos.whirlpool.to_string();

            // ── Bug 2 fix: record fee_growth_global baselines at watch start ──
            // compute_accrued_fees needs (global_now - global_at_watch_start) not
            // (global_now - fee_growth_inside_at_open) which would be a protocol
            // mismatch producing wrong/zero results.
            let init_pool_data =
                rpc.fetch_account_checked(&pool_addr, &whirlpool_program)?;
            let init_pool = protocols::orca::parse_pool(&init_pool_data)?;
            let fee_growth_baseline_a: u128 = init_pool.fee_growth_global_a;
            let fee_growth_baseline_b: u128 = init_pool.fee_growth_global_b;

            // ── Bug 3 fix: persist entry price on first observation ────────────
            // cache::load_entry_price returns None on first run because nothing
            // wrote the cache yet, causing IL to fall back to current price → IL=0.
            // Write it now (at watch start) so every subsequent tick can load it.
            let entry_price_at_start = analytics::greeks::sqrt_q64_to_price(init_pool.sqrt_price)
                * 10f64.powi(9 - 6);
            if cache::load_entry_price(mint).is_none() {
                if let Err(e) = cache::save_entry_price(mint, entry_price_at_start) {
                    tracing::warn!(error = %e, "failed to persist entry price to cache");
                } else {
                    tracing::info!(
                        entry_price = entry_price_at_start,
                        "entry price saved to cache at watch start"
                    );
                }
            }

            let ws_url = cli
                .rpc_url
                .replace("https://", "wss://")
                .replace("http://", "ws://");

            // Connect to Postgres if DATABASE_URL is configured; otherwise run
            // without persistence (preserves dev UX when no DB is available).
            let db_pool: Option<sqlx_postgres::PgPool> = match cli.db_url.as_deref() {
                Some(url) => {
                    let pg = storage::connect(url).await?;
                    storage::run_migrations(&pg).await?;
                    tracing::info!("Storage connected — persisting tick snapshots to Postgres");
                    Some(pg)
                }
                None => {
                    tracing::warn!("DATABASE_URL not set — running watch without persistence");
                    None
                }
            };

            // ── Shadow gate: blocks --live until DB conditions are met (SHADOW-04) ──
            // Gate runs only in Live mode; shadow mode is never gated.
            if matches!(run_mode, RunMode::Live) {
                match &db_pool {
                    None => {
                        eprintln!(
                            "ERROR: shadow gate FAILED: DATABASE_URL required for --live mode"
                        );
                        eprintln!("Hint: set DATABASE_URL and run `cargo run -- watch` (shadow mode) to accumulate ≥14 days of zero-error data before retrying --live.");
                        std::process::exit(2);
                    }
                    Some(pg) => {
                        let status = storage::writer::check_shadow_gate(pg, &pool_addr).await?;
                        if !status.is_pass() {
                            eprintln!("ERROR: {}", status.describe());
                            eprintln!("Hint: run `cargo run -- watch` (shadow mode) and accumulate ≥14 days of zero-error data before retrying --live.");
                            std::process::exit(2);
                        }
                        tracing::info!("shadow gate passed; entering LIVE mode");
                    }
                }
            }

            // ── Risk monitor initialization (RISK-04) ──────────────────────────
            // Load or init risk state from DB. Optional: only when DB is available.
            // Uses Arc<Mutex<RiskMonitor>> because the tick callback is Fn (not FnMut).
            let risk_monitor_opt: Option<
                std::sync::Arc<std::sync::Mutex<strategy::risk_monitor::RiskMonitor>>,
            > = match &db_pool {
                Some(pg) => {
                    let mut risk_state =
                        strategy::risk_monitor::RiskMonitor::load_or_init(pg, &pool_addr).await?;

                    // Log startup halt/pause state before reset so the operator
                    // knows stale flags were detected and are now being cleared (D-12).
                    if risk_state.halt_flag {
                        tracing::warn!(
                            pool = %pool_addr,
                            "risk: halt_flag was active from previous session -- clearing for new session"
                        );
                    }
                    if risk_state.pause_flag {
                        tracing::warn!(
                            pool = %pool_addr,
                            "risk: pause_flag active from previous session -- rebalancing paused"
                        );
                    }
                    if risk_state.operator_pause {
                        tracing::warn!(
                            pool = %pool_addr,
                            "risk: operator_pause active from previous session -- rebalancing paused by operator"
                        );
                    }

                    // Reset session-volatile fields so stale peak_pnl / halt_flag from
                    // a prior session do not produce an instant 100% drawdown halt on
                    // restart. operator_pause is intentionally preserved.
                    strategy::risk_monitor::RiskMonitor::reset_session(pg, &pool_addr).await?;
                    risk_state.peak_pnl = 0.0;
                    risk_state.halt_flag = false;
                    risk_state.current_drawdown_pct = 0.0;
                    tracing::info!(
                        pool = %pool_addr,
                        "risk: session reset -- peak_pnl and halt_flag cleared for new session"
                    );

                    // Derive Drift User PDA from keypair if available (for RISK-03).
                    // In shadow mode (no keypair), drift_user_pubkey = None -> Drift check skipped.
                    let drift_user_pubkey: Option<solana_sdk::pubkey::Pubkey> = None;

                    let monitor = strategy::risk_monitor::RiskMonitor::new(
                        risk_state,
                        max_drawdown_val,
                        max_il_val,
                        drift_min_margin_ratio_val,
                        drift_user_pubkey,
                        cli.rpc_url.clone(),
                    );
                    Some(std::sync::Arc::new(std::sync::Mutex::new(monitor)))
                }
                None => {
                    if max_drawdown_val.is_some()
                        || max_il_val.is_some()
                        || drift_min_margin_ratio_val.is_some()
                    {
                        tracing::warn!(
                            "Risk limits configured but DATABASE_URL not set -- risk monitoring disabled"
                        );
                    }
                    None
                }
            };

            // ── Telegram bot (D-01: integrated tokio task) ──────────────────────
            let pending_approval: std::sync::Arc<
                std::sync::Mutex<Option<tokio::sync::oneshot::Sender<bool>>>,
            > = std::sync::Arc::new(std::sync::Mutex::new(None));

            let _bot_handle: Option<tokio::task::JoinHandle<()>> = if *telegram {
                match (&db_pool, &risk_monitor_opt) {
                    (Some(pg), Some(rm)) => {
                        let chat_id = bot::load_chat_id()?;
                        let bot_state = bot::BotState {
                            db_pool: pg.clone(),
                            risk_monitor: rm.clone(),
                            pool_address: pool_addr.clone(),
                            mint: mint.clone(),
                            pending_approval: pending_approval.clone(),
                            chat_id,
                        };
                        let handle = bot::spawn_bot(bot_state).await?;
                        tracing::info!("Telegram bot started");
                        Some(handle)
                    }
                    _ => {
                        anyhow::bail!(
                            "--telegram requires DATABASE_URL and risk monitor to be active"
                        );
                    }
                }
            } else {
                None
            };

            // ── Telegram proposal bot instance (Plan 02) ───────────────────────
            // A separate Bot handle for sending proposal messages from the tick
            // callback. Distinct from the dispatcher Bot inside spawn_bot() so the
            // watch loop can call send_message without going through the dispatcher.
            let telegram_bot: Option<teloxide::Bot> = if *telegram {
                Some(teloxide::Bot::new(
                    std::env::var("TELEGRAM_BOT_TOKEN")
                        .map_err(|_| anyhow::anyhow!("TELEGRAM_BOT_TOKEN required"))?,
                ))
            } else {
                None
            };
            let telegram_chat_id: Option<i64> = if *telegram {
                Some(bot::load_chat_id()?)
            } else {
                None
            };

            // Capture timeout as an owned value for use in the closure.
            let approve_timeout_secs_val: u64 = *approve_timeout_secs;

            tracing::debug!(approve_timeout_secs_val, "bot approval timeout configured");

            println!("Watching {}  (Ctrl+C to stop)", mint);
            println!("WebSocket: {}", ws_url);

            let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

            // Graceful shutdown on Ctrl+C.
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    let _ = shutdown_tx.send(());
                }
            });

            let pool_addr_clone = pool_addr.clone();
            let rpc_url = cli.rpc_url.clone();
            let rpc_timeout = cli.rpc_timeout;
            let mint_str = mint.clone();
            let on_notify = Box::new(move |json: serde_json::Value| {
                let rpc_inner = rpc::SolanaRpc::with_timeout(&rpc_url, rpc_timeout);
                print!("\x1B[2J\x1B[1;1H");
                println!(
                    "[{}] Pool update received",
                    chrono::Utc::now().format("%H:%M:%S UTC")
                );

                let pool_data =
                    match rpc_inner.fetch_account_checked(&pool_addr_clone, &whirlpool_program) {
                        Ok(d) => d,
                        Err(e) => {
                            tracing::warn!("Failed to fetch pool data: {}", e);
                            return;
                        }
                    };
                let pool = match protocols::orca::parse_pool(&pool_data) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("Failed to parse pool: {}", e);
                        return;
                    }
                };

                // SOL=9 decimals, USDC=6 decimals → multiply raw price by 10^(9-6)=1000
                let price_current = analytics::greeks::sqrt_q64_to_price(pool.sqrt_price)
                    * 10f64.powi(9 - 6);
                let in_range = pool.tick_current_index >= pos.tick_lower_index
                    && pool.tick_current_index <= pos.tick_upper_index;

                println!("Pool:      {}", pool_addr_clone);
                println!("Price:     ${:.4}", price_current);
                println!("Tick:      {}", pool.tick_current_index);
                println!(
                    "In range:  {}",
                    if in_range {
                        "YES"
                    } else {
                        "NO -- needs rebalance"
                    }
                );
                println!("Liquidity: {}", pool.liquidity);

                // ── Real P&L computation (SHADOW-03 / Phase 2) ─────────────────
                // Decimals are not fetched in watch mode; use well-known values for
                // SOL (9) / USDC (6) as a best-effort approximation (same as Raydium
                // position command). Full decimal wiring arrives in Phase 5.
                let scale_a = 10f64.powi(9); // SOL decimals
                let scale_b = 10f64.powi(6); // USDC decimals

                // Bug 2 fix: use fee_growth_baseline_a/b (captured at watch start) as
                // the checkpoint instead of pos.fee_growth_checkpoint_a/b.
                // pos.fee_growth_checkpoint_* tracks fee_growth_inside (protocol value
                // updated on every pool interaction), not fee_growth_global — using it
                // directly produces a protocol mismatch that yields 0 or garbage fees.
                // The baselines give us exactly "fees earned since watch session started".
                let accrued_a = analytics::pnl::compute_accrued_fees(
                    pool.fee_growth_global_a,
                    fee_growth_baseline_a,
                    pos.liquidity,
                );
                let accrued_b = analytics::pnl::compute_accrued_fees(
                    pool.fee_growth_global_b,
                    fee_growth_baseline_b,
                    pos.liquidity,
                );
                tracing::info!(
                    pos_liquidity = pos.liquidity,
                    fee_growth_delta_a = pool.fee_growth_global_a.wrapping_sub(fee_growth_baseline_a),
                    fee_growth_delta_b = pool.fee_growth_global_b.wrapping_sub(fee_growth_baseline_b),
                    fee_owed_a = pos.fee_owed_a,
                    fee_owed_b = pos.fee_owed_b,
                    accrued_a,
                    accrued_b,
                    "fee debug"
                );
                let computed_fees_earned = (pos.fee_owed_a + accrued_a) as f64 / scale_a
                    * price_current
                    + (pos.fee_owed_b + accrued_b) as f64 / scale_b;

                // Bug 3 fix: entry price is now guaranteed to be in cache (written at
                // watch start above). unwrap_or fallback kept as safety net only.
                let entry_price = cache::load_entry_price(&mint_str).unwrap_or(price_current);

                let amounts_result = analytics::amounts::compute_token_amounts(
                    pos.liquidity,
                    pool.sqrt_price,
                    pos.tick_lower_index,
                    pos.tick_upper_index,
                );
                let computed_position_value = match &amounts_result {
                    Ok(a) => {
                        a.amount_a as f64 / scale_a * price_current + a.amount_b as f64 / scale_b
                    }
                    Err(_) => 0.0,
                };

                let price_lower = analytics::greeks::sqrt_q64_to_price(
                    orca_whirlpools_core::tick_index_to_sqrt_price(pos.tick_lower_index),
                );
                let price_upper = analytics::greeks::sqrt_q64_to_price(
                    orca_whirlpools_core::tick_index_to_sqrt_price(pos.tick_upper_index),
                );
                let il_fraction = analytics::pnl::compute_il(
                    entry_price,
                    price_current,
                    price_lower,
                    price_upper,
                );
                let computed_il_usd = il_fraction * computed_position_value;

                // ── Persist tick snapshot + P&L (D-05: pnl write before risk gate) ─
                // pool_ticks write (durable) + pnl_history write (fire-and-forget).
                // Risk gate runs immediately after these writes.
                let snap_opt: Option<storage::writer::PnlSnapshot> = if let Some(ref pg) = db_pool {
                    // Extract Solana slot from the accountNotification context.
                    let slot: i64 = json
                        .pointer("/params/result/context/slot")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);

                    let now = chrono::Utc::now();

                    let tick = storage::writer::PoolTick {
                        pool_address: pool_addr_clone.clone(),
                        slot,
                        tick_current: pool.tick_current_index,
                        sqrt_price: pool.sqrt_price,
                        liquidity: pool.liquidity,
                        fee_growth_global_a: pool.fee_growth_global_a,
                        fee_growth_global_b: pool.fee_growth_global_b,
                        observed_at: now,
                    };

                    // Await write_pool_tick (durability checkpoint). The callback
                    // runs inside a tokio runtime; block_in_place lets us call
                    // block_on without violating single-threaded executor rules.
                    let write_result = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(storage::writer::write_pool_tick(pg, &tick))
                    });
                    if let Err(e) = write_result {
                        tracing::warn!(error = %e, "pool_ticks write failed");
                    }

                    let snap = storage::writer::PnlSnapshot {
                        mint: mint_str.clone(),
                        pool_address: pool_addr_clone.clone(),
                        fees_earned: computed_fees_earned,
                        il_usd: computed_il_usd,
                        net_pnl: computed_fees_earned - computed_il_usd.abs(),
                        position_value: computed_position_value,
                        price: price_current,
                        observed_at: now,
                    };
                    // Fire-and-forget: does not block tick processing (PERSIST-03).
                    std::mem::drop(storage::writer::spawn_pnl_write(pg.clone(), snap.clone()));
                    Some(snap)
                } else {
                    None
                };

                // ── Risk gate (D-04: every tick; D-05: after pnl_write, before should_rebalance) ──
                if let (Some(ref snap), Some(ref risk_arc)) = (&snap_opt, &risk_monitor_opt) {
                    // Fetch Drift margin ratio synchronously (D-01) via block_in_place.
                    // Returns None on RPC failure (treat as "margin OK" per D-03).
                    let drift_margin = {
                        let rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                        // Only fetch if both pubkey and threshold are configured.
                        if rm.drift_user_pubkey.is_some() {
                            tokio::task::block_in_place(|| rm.fetch_drift_margin_ratio())
                        } else {
                            None
                        }
                    };

                    let risk_action = {
                        let mut rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                        rm.evaluate(snap, drift_margin)
                    };

                    let pg_for_persist = db_pool.as_ref().unwrap().clone();

                    match &risk_action {
                        strategy::risk_monitor::RiskAction::HaltAll { drawdown_pct } => {
                            tracing::error!(
                                drawdown_pct,
                                pool = %pool_addr_clone,
                                "risk: drawdown limit breached -- halting all activity"
                            );
                            // LP position close: OrcaExecutor CPI deferred to LIVE-02.
                            // halt_flag is already set in evaluate(); persist it now (D-11).
                            tracing::error!(
                                pool = %pool_addr_clone,
                                "halt: drawdown limit hit -- LP close and Drift hedge close deferred (LIVE-02)"
                            );
                            let state = risk_arc.lock().unwrap_or_else(|p| p.into_inner()).state.clone();
                            strategy::risk_monitor::RiskMonitor::persist_state(
                                pg_for_persist,
                                state,
                            );
                            // Skip rest of tick cycle (D-06): no rebalance evaluation.
                            return;
                        }
                        strategy::risk_monitor::RiskAction::PauseRebalancing { il_pct } => {
                            tracing::warn!(
                                il_pct,
                                pool = %pool_addr_clone,
                                "risk: IL pause active -- skipping rebalance this tick"
                            );
                            let state = risk_arc.lock().unwrap_or_else(|p| p.into_inner()).state.clone();
                            strategy::risk_monitor::RiskMonitor::persist_state(
                                pg_for_persist,
                                state,
                            );
                            // Skip should_rebalance (D-06).
                            return;
                        }
                        strategy::risk_monitor::RiskAction::ResumeRebalancing { il_pct } => {
                            tracing::info!(
                                il_pct,
                                pool = %pool_addr_clone,
                                "risk: IL recovered -- resuming rebalance"
                            );
                            let state = risk_arc.lock().unwrap_or_else(|p| p.into_inner()).state.clone();
                            strategy::risk_monitor::RiskMonitor::persist_state(
                                pg_for_persist,
                                state,
                            );
                            // Fall through to should_rebalance()
                        }
                        strategy::risk_monitor::RiskAction::CloseDriftHedge { margin_ratio } => {
                            tracing::error!(
                                margin_ratio,
                                pool = %pool_addr_clone,
                                "risk: Drift margin below threshold -- Drift hedge close deferred (LIVE-02)"
                            );
                            let state = risk_arc.lock().unwrap_or_else(|p| p.into_inner()).state.clone();
                            strategy::risk_monitor::RiskMonitor::persist_state(
                                pg_for_persist,
                                state,
                            );
                            // LP rebalance continues (RISK-03: only Drift side affected).
                            // Fall through to should_rebalance()
                        }
                        strategy::risk_monitor::RiskAction::Continue => {
                            // Persist state (peak_pnl may have been updated).
                            let state = risk_arc.lock().unwrap_or_else(|p| p.into_inner()).state.clone();
                            strategy::risk_monitor::RiskMonitor::persist_state(
                                pg_for_persist,
                                state,
                            );
                            // Fall through to should_rebalance()
                        }
                    }
                }

                // ── Operator pause gate (D-04) ─────────────────────────────────
                // Check operator_pause AFTER risk gate, BEFORE should_rebalance.
                // This is independent from IL-triggered pause_flag.
                if let Some(ref risk_arc) = &risk_monitor_opt {
                    let rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                    if rm.state.operator_pause {
                        tracing::debug!(pool = %pool_addr_clone, "operator pause active -- skipping rebalance");
                        return;
                    }
                }

                // ── Shadow rebalance decision (SHADOW-02) ───────────────────────
                // Wrap the full decision path so any error sets error_flag=true rather
                // than aborting the tick callback (T-02-05).
                let rebalance_config = strategy::RebalanceConfig::default();
                let decision_result: Result<strategy::RebalanceDecision, String> =
                    Ok(strategy::should_rebalance(
                        pool.tick_current_index,
                        pos.tick_lower_index,
                        pos.tick_upper_index,
                        computed_fees_earned + computed_il_usd,
                        &rebalance_config,
                    ));

                let is_rebalance = matches!(
                    &decision_result,
                    Ok(strategy::RebalanceDecision::Rebalance { .. })
                );

                // Gate: if a rebalance plan were to be submitted, check shadow guard first.
                // In Phase 2 there is no real plan — we use the pool state as the proxy.
                // Real rebalance plan construction arrives in Phase 5.
                if is_rebalance {
                    // ── Telegram approval gate (TG-01, TG-02) ──────────────────
                    // When --telegram is active, send a proposal message and await
                    // /approve within the configured timeout before proceeding.
                    // The callback is a sync Fn so async calls use block_in_place.
                    if let (Some(ref tg_bot), Some(tg_chat)) =
                        (&telegram_bot, telegram_chat_id)
                    {
                        // Build plan to get range_width for the proposal message.
                        let plan = execution::build_rebalance_plan(
                            &mint_str,
                            pos.tick_lower_index,
                            pos.tick_upper_index,
                            pool.tick_spacing as i32,
                        );
                        let trigger_reason = match &decision_result {
                            Ok(strategy::RebalanceDecision::Rebalance { reason }) => {
                                reason.clone()
                            }
                            _ => "unknown".to_string(),
                        };
                        let proposal_data = bot::proposal::ProposalData {
                            pool_address: pool_addr_clone.clone(),
                            trigger_reason,
                            price: price_current,
                            simulated_fees_earned: computed_fees_earned,
                            simulated_il_usd: computed_il_usd,
                            simulated_net_pnl: computed_fees_earned
                                - computed_il_usd.abs(),
                            range_width: (plan.new_tick_upper - plan.new_tick_lower)
                                as f64,
                        };
                        let chat_id = teloxide::types::ChatId(tg_chat);

                        let approved = tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                match bot::proposal::send_proposal(
                                    tg_bot,
                                    chat_id,
                                    &proposal_data,
                                    &pending_approval,
                                )
                                .await
                                {
                                    Ok(rx) => {
                                        bot::proposal::await_approval(
                                            rx,
                                            approve_timeout_secs_val,
                                        )
                                        .await
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "failed to send Telegram proposal; \
                                             skipping approval gate"
                                        );
                                        // Telegram failure: fall through to execute
                                        // without approval so the watch loop is not
                                        // blocked if the bot is temporarily down.
                                        true
                                    }
                                }
                            })
                        });

                        if !approved {
                            // Log skip to DB (TG-02) and continue to next tick.
                            if let Some(ref pg) = db_pool {
                                let _ = tokio::task::block_in_place(|| {
                                    tokio::runtime::Handle::current().block_on(
                                        storage::writer::write_approval_skip(
                                            pg,
                                            &pool_addr_clone,
                                            "timeout",
                                            price_current,
                                        ),
                                    )
                                });
                            }
                            tracing::info!(
                                pool = %pool_addr_clone,
                                "rebalance skipped: approval timeout/rejected"
                            );
                            return;
                        }
                        // Approved: fall through to guard.submit() and execution.
                    }

                    let plan_proxy = format!(
                        "rebalance_needed tick={} range=[{},{}]",
                        pool.tick_current_index, pos.tick_lower_index, pos.tick_upper_index
                    );
                    if let Err(e) = guard.submit(&plan_proxy) {
                        tracing::warn!(error = %e, "rebalance submission gated");
                    }
                }

                // Build and spawn the shadow_rebalances row when a rebalance decision fires.
                // We also write on error so the gate query in Plan 03 can count bad rows.
                if let Some(ref pg) = db_pool {
                    let shadow_row: Option<storage::writer::ShadowRebalanceRow> =
                        match &decision_result {
                            Ok(strategy::RebalanceDecision::Rebalance { reason }) => {
                                let plan = execution::build_rebalance_plan(
                                    &mint_str,
                                    pos.tick_lower_index,
                                    pos.tick_upper_index,
                                    pool.tick_spacing as i32,
                                );
                                let range_width =
                                    (plan.new_tick_upper - plan.new_tick_lower) as f64;
                                tracing::info!(
                                    pool = %pool_addr_clone,
                                    trigger = %reason,
                                    price = price_current,
                                    error = false,
                                    "shadow rebalance decision"
                                );
                                Some(storage::writer::ShadowRebalanceRow {
                                    pool_address: pool_addr_clone.clone(),
                                    trigger_reason: reason.replace(' ', "_"),
                                    price: price_current,
                                    simulated_range_width: Some(range_width),
                                    simulated_fees_earned: Some(computed_fees_earned),
                                    simulated_il_usd: Some(computed_il_usd),
                                    simulated_net_pnl: Some(
                                        computed_fees_earned - computed_il_usd.abs(),
                                    ),
                                    error_flag: false,
                                    error_message: None,
                                })
                            }
                            Ok(strategy::RebalanceDecision::Hold { .. }) => {
                                // No rebalance needed this tick — do not write a row.
                                None
                            }
                            Err(e) => {
                                tracing::error!(
                                    pool = %pool_addr_clone,
                                    error = %e,
                                    "shadow rebalance decision error"
                                );
                                Some(storage::writer::ShadowRebalanceRow {
                                    pool_address: pool_addr_clone.clone(),
                                    trigger_reason: "error".to_string(),
                                    price: price_current,
                                    simulated_range_width: None,
                                    simulated_fees_earned: None,
                                    simulated_il_usd: None,
                                    simulated_net_pnl: None,
                                    error_flag: true,
                                    error_message: Some(e.clone()),
                                })
                            }
                        };

                    if let Some(row) = shadow_row {
                        storage::writer::spawn_shadow_write(pg.clone(), row);
                    }
                }
            });

            data::ws::watch_account(ws_url, pool_addr, shutdown_rx, on_notify).await;
        }
        Commands::Depth { pool } => {
            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let pool_data = rpc.fetch_account_checked(pool, &whirlpool_program)?;
            let pool_state = protocols::orca::parse_pool(&pool_data)?;

            let price_current = analytics::greeks::sqrt_q64_to_price(pool_state.sqrt_price);

            println!(
                "Liquidity Distribution  (pool liquidity: {:.0}M)",
                pool_state.liquidity as f64 / 1e6
            );
            println!("{}", "─".repeat(50));

            for pct in [1.0f64, 2.0, 5.0] {
                let buy = analytics::depth::estimate_impact(
                    price_current,
                    pool_state.liquidity,
                    pct,
                    true,
                );
                let sell = analytics::depth::estimate_impact(
                    price_current,
                    pool_state.liquidity,
                    pct,
                    false,
                );
                println!(
                    "  +{:.0}%  (~${:.4}): ${:.0} needed to buy  |  ${:.0} needed to sell",
                    pct, buy.target_price, buy.usd_needed, sell.usd_needed
                );
            }

            // Fetch surrounding tick arrays and render a depth histogram.
            let whirlpool_pubkey = Pubkey::from_str(pool)?;
            let tick_arrays = protocols::orca::fetch_tick_arrays(
                &rpc,
                &whirlpool_pubkey,
                pool_state.tick_current_index,
                pool_state.tick_spacing,
            )?;

            let mut tick_deltas: Vec<(i32, i128)> = Vec::new();
            for ta in &tick_arrays {
                for (i, tick) in ta.ticks.iter().enumerate() {
                    if tick.initialized {
                        let tick_index =
                            ta.start_tick_index + (i as i32) * (pool_state.tick_spacing as i32);
                        tick_deltas.push((tick_index, tick.liquidity_net));
                    }
                }
            }

            println!();
            println!(
                "Depth Map  ({} initialized ticks across {} arrays)",
                tick_deltas.len(),
                tick_arrays.len()
            );
            println!("{}", "─".repeat(70));

            let distribution = analytics::depth::build_distribution(
                &tick_deltas,
                pool_state.liquidity,
                pool_state.tick_current_index,
                pool_state.tick_spacing as i32,
                8,
            );
            display::table::print_depth_histogram(&distribution, price_current);
        }
        Commands::Impact { pool, size } => {
            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let pool_data = rpc.fetch_account_checked(pool, &whirlpool_program)?;
            let pool_state = protocols::orca::parse_pool(&pool_data)?;

            let price_current = analytics::greeks::sqrt_q64_to_price(pool_state.sqrt_price);

            // Attempt to fetch surrounding tick arrays for a more accurate estimate.
            let whirlpool_pubkey = Pubkey::from_str(pool)?;
            let tick_arrays_result = protocols::orca::fetch_tick_arrays(
                &rpc,
                &whirlpool_pubkey,
                pool_state.tick_current_index,
                pool_state.tick_spacing,
            );

            let (target_price, impact_pct) = match tick_arrays_result {
                Ok(tick_arrays) => {
                    // Build tick delta list from fetched arrays.
                    let mut tick_deltas: Vec<(i32, i128)> = Vec::new();
                    for ta in &tick_arrays {
                        for (i, tick) in ta.ticks.iter().enumerate() {
                            if tick.initialized {
                                let tick_index = ta.start_tick_index
                                    + (i as i32) * (pool_state.tick_spacing as i32);
                                tick_deltas.push((tick_index, tick.liquidity_net));
                            }
                        }
                    }

                    // Build distribution around current tick (16 buckets each side).
                    let distribution = analytics::depth::build_distribution(
                        &tick_deltas,
                        pool_state.liquidity,
                        pool_state.tick_current_index,
                        pool_state.tick_spacing as i32,
                        16,
                    );

                    // Find the bucket containing the current tick (it is the middle bucket).
                    // Walk buckets in the buy direction (ascending price) consuming USD.
                    let mid = distribution.len() / 2;
                    let mut remaining = *size;
                    let mut final_price = price_current;

                    'walk: for bucket in &distribution[mid..] {
                        let l = bucket.liquidity as f64;
                        if l == 0.0 {
                            continue;
                        }
                        // Bucket price is the mid-tick price; use it as the bucket end for the
                        // next bucket step.  For the current bucket we step from price_current.
                        let p_start = final_price;
                        let p_end = bucket.price;
                        if p_end <= p_start {
                            continue;
                        }
                        let sqrt_start = p_start.sqrt();
                        let sqrt_end = p_end.sqrt();
                        // USD to consume this full bucket (buying token A, price rises).
                        let amount_a_full = l * (1.0 / sqrt_start - 1.0 / sqrt_end).abs();
                        let usd_full = amount_a_full * p_start;
                        if remaining <= usd_full {
                            // Trade ends inside this bucket.
                            let amount_a = remaining / p_start;
                            let inv_sqrt_target = 1.0 / sqrt_start - amount_a / l;
                            if inv_sqrt_target > 0.0 {
                                final_price = 1.0 / (inv_sqrt_target * inv_sqrt_target);
                            } else {
                                final_price = f64::INFINITY;
                            }
                            remaining = 0.0;
                            break 'walk;
                        }
                        remaining -= usd_full;
                        final_price = p_end;
                    }

                    if remaining > 0.0 {
                        // Ran out of buckets — trade exhausts sampled liquidity.
                        (f64::INFINITY, f64::INFINITY)
                    } else {
                        let pct = (final_price - price_current) / price_current * 100.0;
                        (final_price, pct)
                    }
                }
                Err(e) => {
                    // Fall back to constant-L approximation.
                    tracing::warn!(
                        "tick-array fetch failed ({}); falling back to constant-L approximation",
                        e
                    );
                    let l = pool_state.liquidity as f64;
                    let sqrt_p = price_current.sqrt();
                    let amount_a = size / price_current;
                    let inv_sqrt_target = (1.0 / sqrt_p) - (amount_a / l);
                    if inv_sqrt_target > 0.0 {
                        let p_target = 1.0 / (inv_sqrt_target * inv_sqrt_target);
                        let pct = (p_target - price_current) / price_current * 100.0;
                        (p_target, pct)
                    } else {
                        (f64::INFINITY, f64::INFINITY)
                    }
                }
            };

            println!("Pool:          {}", pool);
            println!("Current price: ${:.6}", price_current);
            println!("Trade size:    ${:.0}", size);
            if impact_pct.is_finite() {
                println!("Price impact:  {:+.4}%", impact_pct);
                println!("Price after:   ${:.6}", target_price);
            } else {
                println!("Price impact:  > liquidity available");
            }
        }
        Commands::Strategy { command } => match command {
            StrategyCommands::Check {
                mint,
                near_edge_ticks,
                min_pnl,
                entry_price,
            } => {
                use orca_whirlpools_core::tick_index_to_sqrt_price;

                let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
                let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
                let mint_pubkey = Pubkey::from_str(mint)?;
                let (position_pda, _) = Pubkey::find_program_address(
                    &[b"position", mint_pubkey.as_ref()],
                    &whirlpool_program,
                );

                let position_data =
                    rpc.fetch_account_checked(&position_pda.to_string(), &whirlpool_program)?;
                let pos = protocols::orca::parse_position(&position_data)?;

                let pool_data =
                    rpc.fetch_account_checked(&pos.whirlpool.to_string(), &whirlpool_program)?;
                let pool = protocols::orca::parse_pool(&pool_data)?;

                let decimals_a = rpc.fetch_mint_decimals(&pool._token_mint_a)?;
                let decimals_b = rpc.fetch_mint_decimals(&pool._token_mint_b)?;

                let price_current = analytics::greeks::sqrt_q64_to_price(pool.sqrt_price);
                let price_lower = analytics::greeks::sqrt_q64_to_price(tick_index_to_sqrt_price(
                    pos.tick_lower_index,
                ));
                let price_upper = analytics::greeks::sqrt_q64_to_price(tick_index_to_sqrt_price(
                    pos.tick_upper_index,
                ));

                let amounts = analytics::amounts::compute_token_amounts(
                    pos.liquidity,
                    pool.sqrt_price,
                    pos.tick_lower_index,
                    pos.tick_upper_index,
                )?;

                let accrued_a = analytics::pnl::compute_accrued_fees(
                    pool.fee_growth_global_a,
                    pos.fee_growth_checkpoint_a,
                    pos.liquidity,
                );
                let accrued_b = analytics::pnl::compute_accrued_fees(
                    pool.fee_growth_global_b,
                    pos.fee_growth_checkpoint_b,
                    pos.liquidity,
                );

                let scale_a = 10f64.powi(decimals_a as i32);
                let scale_b = 10f64.powi(decimals_b as i32);

                let fees_usd = (pos.fee_owed_a + accrued_a) as f64 / scale_a * price_current
                    + (pos.fee_owed_b + accrued_b) as f64 / scale_b;

                // Resolve entry price: CLI flag takes precedence, then cache, then 0.
                let price_entry = if let Some(ep) = entry_price {
                    cache::save_entry_price(mint, *ep)?;
                    *ep
                } else if let Some(ep) = cache::load_entry_price(mint) {
                    tracing::info!("Loaded cached entry price: ${:.4}", ep);
                    ep
                } else {
                    0.0
                };
                let il_fraction = analytics::pnl::compute_il(
                    price_entry,
                    price_current,
                    price_lower,
                    price_upper,
                );
                let position_value = amounts.amount_a as f64 / scale_a * price_current
                    + amounts.amount_b as f64 / scale_b;
                let il_usd = il_fraction * position_value;
                let net_pnl_usd = fees_usd + il_usd;

                let config = strategy::RebalanceConfig {
                    rebalance_out_of_range: true,
                    near_edge_ticks: *near_edge_ticks,
                    min_net_pnl_usd: *min_pnl,
                };

                let decision = strategy::should_rebalance(
                    pool.tick_current_index,
                    pos.tick_lower_index,
                    pos.tick_upper_index,
                    net_pnl_usd,
                    &config,
                );

                println!("Position:     {}", position_pda);
                println!("Tick current: {}", pool.tick_current_index);
                println!(
                    "Range:        [{}, {}]",
                    pos.tick_lower_index, pos.tick_upper_index
                );
                println!("Net P&L:      ${:.2}", net_pnl_usd);
                match decision {
                    strategy::RebalanceDecision::Hold { reason } => {
                        println!("Decision:     HOLD ({})", reason);
                    }
                    strategy::RebalanceDecision::Rebalance { reason } => {
                        println!("Decision:     REBALANCE ({})", reason);
                    }
                }
            }
        },
        Commands::Db { action } => match action {
            DbAction::Migrate => {
                let db_url = cli
                    .db_url
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("--db-url or DATABASE_URL is required"))?;
                let pool = storage::connect(db_url).await?;
                storage::run_migrations(&pool).await?;
                let repo = storage::positions::PositionsRepo::new(pool);
                let _ = repo.pool();
                println!("Migrations complete");
            }
        },
        Commands::Rebalance { mint, dry_run } => {
            if !*dry_run {
                anyhow::bail!("Only --dry-run is supported");
            }
            let keypair_b58 = std::env::var("LP_INSPECTOR_KEYPAIR").map_err(|_| {
                anyhow::anyhow!(
                    "LP_INSPECTOR_KEYPAIR env var not set (base58 private key required)"
                )
            })?;
            if keypair_b58.trim().is_empty() {
                anyhow::bail!("LP_INSPECTOR_KEYPAIR env var not set (base58 private key required)");
            }

            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let mint_pubkey = Pubkey::from_str(mint)?;
            let (position_pda, _) = Pubkey::find_program_address(
                &[b"position", mint_pubkey.as_ref()],
                &whirlpool_program,
            );

            let position_data =
                rpc.fetch_account_checked(&position_pda.to_string(), &whirlpool_program)?;
            let pos = protocols::orca::parse_position(&position_data)?;

            let pool_data =
                rpc.fetch_account_checked(&pos.whirlpool.to_string(), &whirlpool_program)?;
            let pool = protocols::orca::parse_pool(&pool_data)?;

            let plan = execution::build_rebalance_plan(
                mint,
                pos.tick_lower_index,
                pos.tick_upper_index,
                pool.tick_spacing as i32,
            );
            execution::print_dry_run(&plan);
        }
        Commands::Backtest {
            entry_price,
            price_lower,
            price_upper,
            fee_bps,
            capital,
            days,
            volatility,
            daily_volume,
            position_volume_share,
            tick_spacing,
            rebalance,
            seed,
            pool,
            from,
            to,
            position_liquidity,
            near_edge_ticks,
            range_lower_factor,
            range_upper_factor,
        } => {
            if let Some(pool_addr) = pool {
                // ── DB mode: replay real pool_ticks from TimescaleDB ──────────
                use chrono::NaiveDate;

                let db_url = cli.db_url.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("--db-url or DATABASE_URL is required for DB-mode backtest")
                })?;

                let from_date: NaiveDate = from
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("--from <YYYY-MM-DD> is required in DB mode"))?
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--from must be a valid date (YYYY-MM-DD)"))?;

                let to_date: NaiveDate = to
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("--to <YYYY-MM-DD> is required in DB mode"))?
                    .parse()
                    .map_err(|_| anyhow::anyhow!("--to must be a valid date (YYYY-MM-DD)"))?;

                if to_date <= from_date {
                    anyhow::bail!("--to must be after --from");
                }

                let pg = storage::connect(db_url).await?;
                let ticks =
                    storage::tick_reader::read_ticks(&pg, pool_addr, from_date, to_date).await?;

                if ticks.is_empty() {
                    anyhow::bail!(
                        "No ticks found for pool {} between {} and {}. \
                         Accumulate data with `watch` before running a DB backtest.",
                        pool_addr,
                        from_date,
                        to_date
                    );
                }

                let input = backtest::db_replay::DbBacktestInput {
                    initial_value_usd: *capital,
                    entry_price: *entry_price,
                    price_lower: *price_lower,
                    price_upper: *price_upper,
                    fee_rate_bps: *fee_bps,
                    tick_spacing: *tick_spacing,
                    position_liquidity: *position_liquidity as u128,
                    rebalance_cfg: strategy::RebalanceConfig {
                        rebalance_out_of_range: *rebalance,
                        near_edge_ticks: *near_edge_ticks,
                        min_net_pnl_usd: 0.0,
                    },
                    range_factor_lower: *range_lower_factor,
                    range_factor_upper: *range_upper_factor,
                };

                let result = backtest::db_replay::run_db_backtest(input, &ticks)?;
                println!("Backtest (DB mode) — {} ticks replayed", ticks.len());
                backtest::print_results(&result);
            } else {
                // ── GBM mode: synthetic price path simulation ─────────────────
                let params = backtest::BacktestParams {
                    entry_price: *entry_price,
                    price_lower: *price_lower,
                    price_upper: *price_upper,
                    fee_rate_bps: *fee_bps,
                    initial_value_usd: *capital,
                    days: *days,
                    annual_volatility: *volatility,
                    daily_volume_usd: *daily_volume,
                    position_volume_share: *position_volume_share,
                    tick_spacing: *tick_spacing,
                    strategy_rebalance: *rebalance,
                };
                let result = backtest::run(&params, *seed);
                backtest::print_results(&result);
            }
        }
        Commands::Hedge { mint, dry_run } => {
            if !*dry_run {
                anyhow::bail!("Only --dry-run is supported");
            }
            let keypair_b58 = std::env::var("LP_INSPECTOR_KEYPAIR").map_err(|_| {
                anyhow::anyhow!(
                    "LP_INSPECTOR_KEYPAIR env var not set (base58 private key required)"
                )
            })?;
            if keypair_b58.trim().is_empty() {
                anyhow::bail!("LP_INSPECTOR_KEYPAIR env var not set (base58 private key required)");
            }

            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let mint_pubkey = Pubkey::from_str(mint)?;
            let (position_pda, _) = Pubkey::find_program_address(
                &[b"position", mint_pubkey.as_ref()],
                &whirlpool_program,
            );

            let position_data =
                rpc.fetch_account_checked(&position_pda.to_string(), &whirlpool_program)?;
            let pos = protocols::orca::parse_position(&position_data)?;

            let pool_data =
                rpc.fetch_account_checked(&pos.whirlpool.to_string(), &whirlpool_program)?;
            let pool = protocols::orca::parse_pool(&pool_data)?;

            let price_current = analytics::greeks::sqrt_q64_to_price(pool.sqrt_price);
            let greeks = analytics::greeks::compute_greeks(
                pos.liquidity,
                pool.sqrt_price,
                pos.tick_lower_index,
                pos.tick_upper_index,
            );

            let mut plan = execution::compute_hedge_size(greeks.delta, price_current);
            plan.position_mint = mint.clone();
            execution::print_hedge_dry_run(&plan);
        }
    }

    Ok(())
}
