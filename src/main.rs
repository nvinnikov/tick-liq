use anyhow::Result;
use clap::{Parser, Subcommand};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

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

/// Decode the raw account bytes carried in an `accountSubscribe` notification.
///
/// The subscription uses `encoding: base64`, so the pushed frame already
/// contains the full account at `/params/result/value/data` as
/// `[base64_string, "base64"]`. Decoding it avoids a redundant HTTP
/// `getAccount` per tick and guarantees the recorded `context.slot` matches
/// the state we act on (an HTTP refetch could land on a later slot).
/// Returns `None` if the field is absent or not valid base64.
fn account_data_from_notification(json: &serde_json::Value) -> Option<Vec<u8>> {
    use base64::Engine;
    let b64 = json.pointer("/params/result/value/data/0")?.as_str()?;
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

/// Resolve a position's entry price: an explicit `--entry-price` flag wins (and
/// is cached for later runs), else the cached value, else 0.0 ("unknown", which
/// `compute_il` treats as zero IL). Shared by `position` / `strategy check`.
fn resolve_entry_price(mint: &str, flag: &Option<f64>) -> Result<f64> {
    if let Some(ep) = flag {
        cache::save_entry_price(mint, *ep)?;
        Ok(*ep)
    } else if let Some(ep) = cache::load_entry_price(mint) {
        tracing::info!("Loaded cached entry price: ${:.4}", ep);
        Ok(ep)
    } else {
        Ok(0.0)
    }
}

/// Derive the Orca position PDA from its mint, then fetch + parse the position
/// and its pool (owner-verified). Shared by every command that reads an Orca
/// position (position / strategy check / rebalance / hedge / watch).
fn load_orca_position_and_pool(
    rpc: &rpc::SolanaRpc,
    mint: &str,
) -> Result<(
    Pubkey,
    protocols::orca::WhirlpoolPosition,
    protocols::orca::WhirlpoolPool,
)> {
    let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
    let mint_pubkey = Pubkey::from_str(mint)?;
    let (position_pda, _) =
        Pubkey::find_program_address(&[b"position", mint_pubkey.as_ref()], &whirlpool_program);
    let position_data = rpc.fetch_account_checked(&position_pda.to_string(), &whirlpool_program)?;
    let pos = protocols::orca::parse_position(&position_data)?;
    let pool_data = rpc.fetch_account_checked(&pos.whirlpool.to_string(), &whirlpool_program)?;
    let pool = protocols::orca::parse_pool(&pool_data)?;
    Ok((position_pda, pos, pool))
}

/// Unit-correct P&L snapshot for an Orca position. This is the single place
/// that converts raw sqrt prices to UI prices and computes fees / IL / value,
/// so `position` and `strategy check` cannot drift apart on the (historically
/// bug-prone) unit handling.
struct OrcaPnl {
    position_pda: Pubkey,
    pos: protocols::orca::WhirlpoolPosition,
    pool: protocols::orca::WhirlpoolPool,
    decimals_a: u8,
    decimals_b: u8,
    price_current: f64,
    price_lower: f64,
    price_upper: f64,
    in_range: bool,
    amounts: analytics::amounts::TokenAmounts,
    fees_usd: f64,
    il_usd: f64,
    position_value: f64,
    net_pnl_usd: f64,
}

fn compute_orca_pnl(
    rpc: &rpc::SolanaRpc,
    mint: &str,
    entry_price: &Option<f64>,
) -> Result<OrcaPnl> {
    use analytics::greeks::sqrt_q64_to_ui_price;
    use orca_whirlpools_core::tick_index_to_sqrt_price;

    let (position_pda, pos, pool) = load_orca_position_and_pool(rpc, mint)?;

    let decimals_a = rpc.fetch_mint_decimals(&pool._token_mint_a)?;
    let decimals_b = rpc.fetch_mint_decimals(&pool._token_mint_b)?;

    // UI prices: decimal-adjusted so they share a unit space with --entry-price,
    // the entry-price cache and the scaled token amounts.
    let price_current = sqrt_q64_to_ui_price(pool.sqrt_price, decimals_a, decimals_b);
    let price_lower = sqrt_q64_to_ui_price(
        tick_index_to_sqrt_price(pos.tick_lower_index),
        decimals_a,
        decimals_b,
    );
    let price_upper = sqrt_q64_to_ui_price(
        tick_index_to_sqrt_price(pos.tick_upper_index),
        decimals_a,
        decimals_b,
    );

    let in_range = pool.tick_current_index >= pos.tick_lower_index
        && pool.tick_current_index <= pos.tick_upper_index;

    let amounts = analytics::amounts::compute_token_amounts(
        pos.liquidity,
        pool.sqrt_price,
        pos.tick_lower_index,
        pos.tick_upper_index,
    )?;

    let scale_a = 10f64.powi(decimals_a as i32);
    let scale_b = 10f64.powi(decimals_b as i32);

    // Fees: on-chain fee_owed only. pos.fee_growth_checkpoint_* is a
    // fee_growth_INSIDE snapshot — pairing it with fee_growth_global in
    // compute_accrued_fees is the "Bug 2" protocol mismatch that yields garbage.
    // A one-shot command has no session baseline, and real fee_growth_inside
    // needs tick-array data, so the collectible owed amount is the honest figure.
    let fees_usd =
        pos.fee_owed_a as f64 / scale_a * price_current + pos.fee_owed_b as f64 / scale_b;

    let price_entry = resolve_entry_price(mint, entry_price)?;
    let il_fraction =
        analytics::pnl::compute_il(price_entry, price_current, price_lower, price_upper);
    let position_value =
        amounts.amount_a as f64 / scale_a * price_current + amounts.amount_b as f64 / scale_b;
    let il_usd = il_fraction * position_value;
    let net_pnl_usd = fees_usd + il_usd;

    Ok(OrcaPnl {
        position_pda,
        pos,
        pool,
        decimals_a,
        decimals_b,
        price_current,
        price_lower,
        price_upper,
        in_range,
        amounts,
        fees_usd,
        il_usd,
        position_value,
        net_pnl_usd,
    })
}

/// Validate that the signing keypair is configured (presence only; real base58
/// decoding + signing arrives in Phase 5). Shared by rebalance / hedge.
fn require_keypair_b58() -> Result<String> {
    let kp = std::env::var("LP_INSPECTOR_KEYPAIR").map_err(|_| {
        anyhow::anyhow!("LP_INSPECTOR_KEYPAIR env var not set (base58 private key required)")
    })?;
    if kp.trim().is_empty() {
        anyhow::bail!("LP_INSPECTOR_KEYPAIR env var not set (base58 private key required)");
    }
    Ok(kp)
}

/// Flatten initialized ticks from fetched TickArrays into `(tick_index,
/// liquidity_net)` pairs for `build_distribution`. Shared by depth / impact.
fn extract_tick_deltas(
    tick_arrays: &[protocols::orca::TickArray],
    tick_spacing: u16,
) -> Vec<(i32, i128)> {
    let mut out = Vec::new();
    for ta in tick_arrays {
        for (i, tick) in ta.ticks.iter().enumerate() {
            if tick.initialized {
                out.push((
                    ta.start_tick_index + (i as i32) * (tick_spacing as i32),
                    tick.liquidity_net,
                ));
            }
        }
    }
    out
}

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
        /// Drift wallet authority (base58 pubkey) that owns the Drift User
        /// account to monitor. Required to activate --drift-min-margin-ratio;
        /// without it the margin check stays disabled (no keypair is needed —
        /// monitoring is read-only).
        #[arg(long)]
        drift_authority: Option<String>,
        /// Enable Telegram bot for rebalance approvals and operator commands.
        /// Requires TELEGRAM_BOT_TOKEN env var.
        #[arg(long)]
        telegram: bool,
        /// Telegram approval timeout in seconds (default 300 = 5 min).
        /// Rebalance is skipped if /approve not received within this window.
        #[arg(long, default_value_t = 300u64)]
        approve_timeout_secs: u64,
        /// Entry price override (USD). If provided, unconditionally saves this as the
        /// cached entry price instead of using the current pool price at watch start.
        #[arg(long)]
        entry_price: Option<f64>,
        /// CEX symbol for Binance @bookTicker price feed (e.g. SOLUSDT).
        /// When set, Binance mid-price replaces on-chain sqrt_price for IL / P&L /
        /// rebalance-signal P&L gate. On-chain `tick_current_index` is still used for
        /// range-boundary checks. When absent, on-chain price is used throughout
        /// (existing Phase 2 behavior).
        #[arg(long)]
        cex_symbol: Option<String>,
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
        /// Token A mint decimals (DB mode; bridges UI prices and raw ticks)
        #[arg(long, default_value_t = 9u8)]
        decimals_a: u8,
        /// Token B mint decimals (DB mode)
        #[arg(long, default_value_t = 6u8)]
        decimals_b: u8,
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
    /// Backfill real historical pool_ticks from GeckoTerminal OHLCV (no API key).
    ///
    /// Pulls price + volume history for a Solana pool and synthesises pool_ticks
    /// rows so `backtest --pool` can replay real data. Use --dry-run to fetch and
    /// preview without a database.
    Backfill {
        /// Pool address (e.g. Orca SOL/USDC Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE)
        #[arg(long)]
        pool: String,
        /// Start date (YYYY-MM-DD, inclusive, UTC)
        #[arg(long)]
        from: String,
        /// End date (YYYY-MM-DD, exclusive, UTC)
        #[arg(long)]
        to: String,
        /// OHLCV timeframe: day | hour | minute
        #[arg(long, default_value = "day")]
        timeframe: String,
        /// Pool fee rate in basis points (e.g. 4 = 0.04%)
        #[arg(long, default_value_t = 4.0)]
        fee_bps: f64,
        /// Constant pool liquidity estimate (Q64.64 L). Read from `depth`/on-chain.
        #[arg(long)]
        pool_liquidity: u128,
        /// Token A mint decimals (SOL = 9)
        #[arg(long, default_value_t = 9u8)]
        decimals_a: u8,
        /// Token B mint decimals (USDC = 6)
        #[arg(long, default_value_t = 6u8)]
        decimals_b: u8,
        /// Tick spacing of the pool
        #[arg(long, default_value_t = 64i32)]
        tick_spacing: i32,
        /// Fetch + synthesise + preview only; do not write to the database
        #[arg(long)]
        dry_run: bool,
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
                    let p = compute_orca_pnl(&rpc, mint, entry_price)?;

                    // Display-only extras (not part of the shared P&L core).
                    let symbol_a = rpc.fetch_token_symbol(&p.pool._token_mint_a);
                    let symbol_b = rpc.fetch_token_symbol(&p.pool._token_mint_b);
                    let range_pct = if p.in_range && (p.price_upper - p.price_lower) > 0.0 {
                        (p.price_current - p.price_lower) / (p.price_upper - p.price_lower) * 100.0
                    } else {
                        0.0
                    };
                    let greeks = analytics::greeks::compute_greeks(
                        p.pos.liquidity,
                        p.pool.sqrt_price,
                        p.pos.tick_lower_index,
                        p.pos.tick_upper_index,
                    );

                    let summary = display::table::PositionSummary {
                        protocol: "Orca".to_string(),
                        pool_address: p.pos.whirlpool.to_string(),
                        fee_rate_bps: p.pool.fee_rate as f64 / 100.0,
                        price_lower: p.price_lower,
                        price_upper: p.price_upper,
                        price_current: p.price_current,
                        in_range: p.in_range,
                        range_pct,
                        decimals_a: p.decimals_a,
                        decimals_b: p.decimals_b,
                        symbol_a,
                        symbol_b,
                        pnl: analytics::pnl::PnlResult {
                            fees_usd: p.fees_usd,
                            il_usd: p.il_usd,
                            net_usd: p.net_pnl_usd,
                            initial_value_usd: p.position_value,
                        },
                        greeks,
                        amounts: p.amounts,
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

                    // Raydium PoolState carries the mint decimals directly.
                    let decimals_a: u8 = pool._mint_decimals_0;
                    let decimals_b: u8 = pool._mint_decimals_1;

                    let price_current = analytics::greeks::sqrt_q64_to_ui_price(
                        pool.sqrt_price_x64,
                        decimals_a,
                        decimals_b,
                    );
                    let price_lower = analytics::greeks::sqrt_q64_to_ui_price(
                        tick_index_to_sqrt_price(pos.tick_lower_index),
                        decimals_a,
                        decimals_b,
                    );
                    let price_upper = analytics::greeks::sqrt_q64_to_ui_price(
                        tick_index_to_sqrt_price(pos.tick_upper_index),
                        decimals_a,
                        decimals_b,
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

                    let price_entry = resolve_entry_price(mint, entry_price)?;
                    let il_fraction = analytics::pnl::compute_il(
                        price_entry,
                        price_current,
                        price_lower,
                        price_upper,
                    );

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
                        protocol: "Raydium".to_string(),
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
            drift_authority,
            telegram,
            approve_timeout_secs,
            entry_price,
            cex_symbol,
        } => {
            match &cex_symbol {
                Some(sym) => {
                    if sym.trim().is_empty() {
                        anyhow::bail!("--cex-symbol must not be empty");
                    }
                    tracing::info!("cex_ws: Binance feed will start for {}", sym);
                }
                None => tracing::info!("--cex-symbol not set, using on-chain price"),
            }

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

            // Resolve the Drift User PDA from the operator-supplied wallet
            // authority. Read-only monitoring, so no keypair is required; the
            // margin check only runs when BOTH a threshold and an authority are
            // given. --drift-authority without a threshold (or vice versa) is a
            // misconfiguration worth failing fast on.
            let drift_user_pubkey: Option<solana_sdk::pubkey::Pubkey> = match drift_authority {
                Some(auth) => {
                    let authority = Pubkey::from_str(auth).map_err(|e| {
                        anyhow::anyhow!("--drift-authority is not a valid pubkey: {}", e)
                    })?;
                    if drift_min_margin_ratio.is_none() {
                        anyhow::bail!(
                            "--drift-authority requires --drift-min-margin-ratio to be set"
                        );
                    }
                    Some(strategy::risk_monitor::RiskMonitor::derive_drift_user_pda(
                        &authority,
                    ))
                }
                None => {
                    if drift_min_margin_ratio.is_some() {
                        tracing::warn!(
                            "--drift-min-margin-ratio set without --drift-authority -- Drift margin check is DISABLED (no account to read)"
                        );
                    }
                    None
                }
            };

            let max_drawdown_val: Option<f64> = *max_drawdown;
            let max_il_val: Option<f64> = *max_il;
            let drift_min_margin_ratio_val: Option<f64> = *drift_min_margin_ratio;

            // ShadowGuard is the single source of truth for shadow/live mode.
            let guard = if *live {
                execution::ShadowGuard::live()
            } else {
                execution::ShadowGuard::shadow()
            };
            tracing::info!(mode = ?guard, "watch starting");
            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            // Whirlpool program id is also needed later for the per-tick pool /
            // position refetch inside the processor task.
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let (position_pda, pos, init_pool) = load_orca_position_and_pool(&rpc, mint)?;
            let pool_addr = pos.whirlpool.to_string();

            // ── Bug 2 fix: record fee_growth_global baselines at watch start ──
            // compute_accrued_fees needs (global_now - global_at_watch_start) not
            // (global_now - fee_growth_inside_at_open) which would be a protocol
            // mismatch producing wrong/zero results.
            let fee_growth_baseline_a: u128 = init_pool.fee_growth_global_a;
            let fee_growth_baseline_b: u128 = init_pool.fee_growth_global_b;

            // Real token decimals — fetched once at watch start. Every price,
            // fee and P&L figure below (including rows persisted to Postgres
            // and the risk-gate inputs) depends on these; hardcoding SOL/USDC
            // 9/6 silently corrupts all of it for any other pair.
            let decimals_a = rpc.fetch_mint_decimals(&init_pool._token_mint_a)?;
            let decimals_b = rpc.fetch_mint_decimals(&init_pool._token_mint_b)?;

            // Validate entry price if provided
            if let Some(ep) = entry_price {
                if *ep <= 0.0 {
                    anyhow::bail!("--entry-price must be positive (got {})", ep);
                }
            }

            // --entry-price override: unconditionally persist operator-supplied price
            // so the Bug 3 guard (which checks is_none()) will skip the pool-price fallback.
            if let Some(ep) = entry_price {
                cache::save_entry_price(mint, *ep)?;
                tracing::info!(
                    entry_price = *ep,
                    "entry price overridden via --entry-price flag"
                );
            }

            // ── Bug 3 fix: persist entry price on first observation ────────────
            // cache::load_entry_price returns None on first run because nothing
            // wrote the cache yet, causing IL to fall back to current price → IL=0.
            // Write it now (at watch start) so every subsequent tick can load it.
            let entry_price_at_start = analytics::greeks::sqrt_q64_to_ui_price(
                init_pool.sqrt_price,
                decimals_a,
                decimals_b,
            );
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
            let entry_price_for_watch =
                cache::load_entry_price(mint).unwrap_or(entry_price_at_start);

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
            if !guard.is_shadow() {
                match &db_pool {
                    None => {
                        eprintln!(
                            "ERROR: shadow gate FAILED: DATABASE_URL required for --live mode"
                        );
                        eprintln!(
                            "Hint: set DATABASE_URL and run `cargo run -- watch` (shadow mode) to accumulate ≥14 days of zero-error data before retrying --live."
                        );
                        std::process::exit(2);
                    }
                    Some(pg) => {
                        let status = storage::writer::check_shadow_gate(pg, &pool_addr).await?;
                        if !status.is_pass() {
                            eprintln!("ERROR: {}", status.describe());
                            eprintln!(
                                "Hint: run `cargo run -- watch` (shadow mode) and accumulate ≥14 days of zero-error data before retrying --live."
                            );
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

                    // Log startup halt/pause state so the operator knows which
                    // persistent flags carry over into this session (D-12).
                    if risk_state.halt_flag {
                        tracing::error!(
                            pool = %pool_addr,
                            "risk: halt_flag active from previous session -- rebalancing remains halted until cleared via SQL (D-12)"
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

                    // Reset session-volatile fields so a stale peak_pnl from a
                    // prior session does not read as an instant 100% drawdown.
                    // halt_flag is NOT cleared: a drawdown halt is a kill-switch
                    // that must survive restarts until the operator clears it
                    // via SQL (D-12), same as operator_pause.
                    strategy::risk_monitor::RiskMonitor::reset_session(pg, &pool_addr).await?;
                    risk_state.peak_pnl = 0.0;
                    risk_state.current_drawdown_pct = 0.0;
                    tracing::info!(
                        pool = %pool_addr,
                        "risk: session reset -- peak_pnl/drawdown cleared (halt_flag preserved)"
                    );

                    // Drift User PDA resolved above from --drift-authority (read-only
                    // monitoring; None disables the per-tick margin check).
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
            let pending_approval: bot::PendingApprovalSlot =
                std::sync::Arc::new(std::sync::Mutex::new(None));

            let bot_handle: Option<tokio::task::JoinHandle<()>> = if *telegram {
                match (&db_pool, &risk_monitor_opt) {
                    (Some(pg), Some(rm)) => {
                        let chat_id = bot::load_chat_id()?;
                        let allowed_user_ids = bot::load_allowed_user_ids()?;
                        let bot_state = bot::BotState {
                            db_pool: pg.clone(),
                            risk_monitor: rm.clone(),
                            pool_address: pool_addr.clone(),
                            mint: mint.clone(),
                            pending_approval: pending_approval.clone(),
                            chat_id,
                            allowed_user_ids,
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
            // Redact any api-key/password before printing (the RPC URL may embed
            // a secret in its query string or userinfo).
            println!("WebSocket: {}", data::ws::redact_url(&ws_url));

            let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

            // Phase 11: CEX price feed (D-01..D-07).
            // State is shared between the Binance WS task (writer) and the on_notify
            // closure (reader). We use std::sync::RwLock so the sync closure can read
            // without block_in_place. The writer holds the lock only briefly and never
            // across .await points.
            let cex_price_state: data::cex_ws::CexPriceState =
                std::sync::Arc::new(std::sync::RwLock::new(None));

            let cex_handle = if let Some(sym) = cex_symbol.as_ref() {
                let sym_owned = sym.clone();
                let state_clone = std::sync::Arc::clone(&cex_price_state);
                let cex_shutdown = shutdown_tx.subscribe();
                Some(tokio::spawn(async move {
                    data::cex_ws::watch_binance_price(sym_owned, state_clone, cex_shutdown).await;
                    // `watch_binance_price` only returns on shutdown (broadcast
                    // fired or channel closed), so this log fires on graceful
                    // exit. Keep it at info! — warn! misled re-review (IN-04).
                    tracing::info!("cex_ws: feed task exited cleanly");
                }))
            } else {
                None
            };

            // Graceful shutdown on Ctrl+C.
            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    let _ = shutdown_tx.send(());
                }
            });

            // Tracks whether we are currently in the stale state, to emit warn only on transitions.
            let cex_was_stale = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cex_was_stale_closure = std::sync::Arc::clone(&cex_was_stale);

            let cex_price_state_closure = std::sync::Arc::clone(&cex_price_state);

            let pool_addr_clone = pool_addr.clone();
            let position_pda_str = position_pda.to_string();
            let rpc_url = cli.rpc_url.clone();
            let rpc_timeout = cli.rpc_timeout;
            let mint_str = mint.clone();

            // Tick pipeline: the WS callback must stay cheap — anything slow
            // inside it (blocking RPC, DB writes, the Telegram approval wait)
            // parks the WS select loop, starving ping/pong until the server
            // drops the connection. Notifications flow through a bounded
            // channel to a dedicated processor task; if the processor falls
            // behind, the oldest pending update is the one we skip.
            let (tick_tx, mut tick_rx) = tokio::sync::mpsc::channel::<serde_json::Value>(64);

            let processor = tokio::spawn(async move {
                // One RPC client for the whole session (constructing a fresh
                // reqwest client per tick costs a TCP+TLS handshake each time).
                let rpc_inner = rpc::SolanaRpc::with_timeout(&rpc_url, rpc_timeout);
                // Startup position snapshot, refreshed from chain each tick so a
                // mid-session fee collect or liquidity change is reflected in
                // fee_owed / liquidity (the position account is not part of the
                // pool subscription, so it must be polled).
                let mut current_pos = pos;
                while let Some(json) = tick_rx.recv().await {
                    // Refresh the position; on failure keep the last-known
                    // snapshot rather than dropping the whole tick.
                    match tokio::task::block_in_place(|| {
                        rpc_inner.fetch_account_checked(&position_pda_str, &whirlpool_program)
                    }) {
                        Ok(d) => match protocols::orca::parse_position(&d) {
                            Ok(p) => current_pos = p,
                            Err(e) => {
                                tracing::warn!("position reparse failed; using last-known: {}", e)
                            }
                        },
                        Err(e) => {
                            tracing::warn!("position refetch failed; using last-known: {}", e)
                        }
                    }
                    let pos = &current_pos;

                    // `async` block so the body's per-tick `return`s keep their
                    // old "skip this tick" semantics.
                    #[allow(clippy::redundant_async_block)]
                    async {
                print!("\x1B[2J\x1B[1;1H");
                println!(
                    "[{}] Pool update received",
                    chrono::Utc::now().format("%H:%M:%S UTC")
                );

                // Pool state: prefer the account bytes already in THIS
                // notification (so the state matches the slot we record below),
                // falling back to an HTTP fetch only when the payload is absent.
                let pool = match account_data_from_notification(&json) {
                    Some(bytes) => match protocols::orca::parse_pool(&bytes) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!("Failed to parse notification pool data: {}", e);
                            return;
                        }
                    },
                    None => {
                        // The solana RpcClient is blocking; block_in_place keeps
                        // it from starving this worker's peers (we are on the
                        // processor task, so it no longer stalls the WS loop).
                        let pool_data = match tokio::task::block_in_place(|| {
                            rpc_inner.fetch_account_checked(&pool_addr_clone, &whirlpool_program)
                        }) {
                            Ok(d) => d,
                            Err(e) => {
                                tracing::warn!("Failed to fetch pool data: {}", e);
                                return;
                            }
                        };
                        match protocols::orca::parse_pool(&pool_data) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::warn!("Failed to parse pool: {}", e);
                                return;
                            }
                        }
                    }
                };

                // Phase 11: Resolve current price from CEX feed, falling back to on-chain
                // sqrt_price when CEX data is stale (>30s) or not yet received.
                // NOTE: tick_current_index (used by should_rebalance) is NOT affected —
                // range-boundary checks stay on-chain per D-07 + RESEARCH Pitfall 5.
                const CEX_STALE_SECS: u64 = 30;

                let onchain_price =
                    analytics::greeks::sqrt_q64_to_ui_price(pool.sqrt_price, decimals_a, decimals_b);
                let price_current: f64 = {
                    use std::sync::atomic::Ordering;
                    let guard = cex_price_state_closure
                        .read()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match guard.as_ref() {
                        Some(cp) if cp.updated_at.elapsed().as_secs() < CEX_STALE_SECS => {
                            if cex_was_stale_closure.swap(false, Ordering::SeqCst) {
                                tracing::info!("cex_ws: price fresh again, resuming CEX feed");
                            }
                            cp.price
                        }
                        Some(_) => {
                            if !cex_was_stale_closure.swap(true, Ordering::SeqCst) {
                                tracing::warn!(
                                    "cex_ws: price stale >{}s, falling back to on-chain sqrt_price",
                                    CEX_STALE_SECS
                                );
                            }
                            onchain_price
                        }
                        None => onchain_price,
                    }
                };
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
                let scale_a = 10f64.powi(decimals_a as i32);
                let scale_b = 10f64.powi(decimals_b as i32);

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

                let computed_fees_earned = (pos.fee_owed_a + accrued_a) as f64 / scale_a
                    * price_current
                    + (pos.fee_owed_b + accrued_b) as f64 / scale_b;

                let entry_price = entry_price_for_watch;

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

                // BUG-qr9 fix: range bounds in the same UI unit space as
                // entry/current — mixed units make the clamp collapse IL to ~0.
                let price_lower = analytics::greeks::sqrt_q64_to_ui_price(
                    orca_whirlpools_core::tick_index_to_sqrt_price(pos.tick_lower_index),
                    decimals_a,
                    decimals_b,
                );
                let price_upper = analytics::greeks::sqrt_q64_to_ui_price(
                    orca_whirlpools_core::tick_index_to_sqrt_price(pos.tick_upper_index),
                    decimals_a,
                    decimals_b,
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
                let snap_opt: Option<storage::writer::PnlSnapshot> =
                    if let Some(pg) = db_pool.as_ref() {
                        let now = chrono::Utc::now();

                        // pool_ticks is keyed by (pool_address, slot). A missing
                        // context.slot must NOT default to 0 — with ON CONFLICT
                        // DO NOTHING that would silently drop every tick after
                        // the first. Skip the durability write for this tick
                        // instead (P&L / risk below still run).
                        match json
                            .pointer("/params/result/context/slot")
                            .and_then(|v| v.as_i64())
                        {
                            Some(slot) => {
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
                                // Awaited on the processor task (no longer the WS
                                // loop), so this is the durability checkpoint.
                                if let Err(e) = storage::writer::write_pool_tick(pg, &tick).await {
                                    tracing::warn!(error = %e, "pool_ticks write failed");
                                }
                            }
                            None => tracing::warn!(
                                "notification missing context.slot; skipping pool_ticks write this tick"
                            ),
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
                        // Awaited (not fire-and-forget): a detached task would be
                        // cancelled by runtime drop on shutdown, losing the row.
                        if let Err(e) = storage::writer::write_pnl_snapshot(pg, &snap).await {
                            tracing::warn!(error = %e, mint = %snap.mint, "pnl write failed");
                        }
                        Some(snap)
                    } else {
                        None
                    };

                // ── Risk gate (D-04: every tick; D-05: after pnl_write, before should_rebalance) ──
                if let (Some(snap), Some(risk_arc)) = (&snap_opt, &risk_monitor_opt) {
                    // Fetch Drift margin ratio synchronously (D-01). Snapshot the
                    // inputs under the lock and RELEASE it before the blocking
                    // RPC, so a slow/retrying Drift fetch can't stall the
                    // Telegram bot handlers that lock the same RiskMonitor mutex.
                    // Returns None on RPC failure (treat as "margin OK" per D-03).
                    let drift_inputs = {
                        let rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                        rm.drift_user_pubkey.map(|pk| (rm.rpc_url.clone(), pk))
                    };
                    let drift_margin = match drift_inputs {
                        Some((url, pk)) => tokio::task::block_in_place(|| {
                            strategy::risk_monitor::RiskMonitor::fetch_drift_margin_ratio_for(
                                &url, pk,
                            )
                        }),
                        None => None,
                    };

                    // Durable fields before evaluate — used to debounce persist.
                    // current_drawdown_pct is derived and shifts every tick, so
                    // it is deliberately excluded (persisting it each tick was a
                    // redundant DB write per tick).
                    let before = {
                        let rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                        (
                            rm.state.peak_pnl,
                            rm.state.pause_flag,
                            rm.state.halt_flag,
                            rm.state.operator_pause,
                        )
                    };

                    let risk_action = {
                        let mut rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                        rm.evaluate(snap, drift_margin)
                    };

                    let halt_tick = match &risk_action {
                        strategy::risk_monitor::RiskAction::HaltAll { drawdown_pct } => {
                            tracing::error!(
                                drawdown_pct,
                                pool = %pool_addr_clone,
                                "risk: drawdown limit breached -- halting all activity"
                            );
                            tracing::error!(
                                pool = %pool_addr_clone,
                                "halt: drawdown limit hit -- LP close and Drift hedge close deferred (LIVE-02)"
                            );
                            true
                        }
                        strategy::risk_monitor::RiskAction::PauseRebalancing { il_pct } => {
                            tracing::warn!(
                                il_pct,
                                pool = %pool_addr_clone,
                                "risk: IL pause active -- skipping rebalance this tick"
                            );
                            true
                        }
                        strategy::risk_monitor::RiskAction::ResumeRebalancing { il_pct } => {
                            tracing::info!(
                                il_pct,
                                pool = %pool_addr_clone,
                                "risk: IL recovered -- resuming rebalance"
                            );
                            false
                        }
                        strategy::risk_monitor::RiskAction::CloseDriftHedge { margin_ratio } => {
                            tracing::error!(
                                margin_ratio,
                                pool = %pool_addr_clone,
                                "risk: Drift margin below threshold -- Drift hedge close deferred (LIVE-02)"
                            );
                            false
                        }
                        strategy::risk_monitor::RiskAction::Continue => false,
                    };

                    let (risk_state, changed) = {
                        let rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                        let after = (
                            rm.state.peak_pnl,
                            rm.state.pause_flag,
                            rm.state.halt_flag,
                            rm.state.operator_pause,
                        );
                        (rm.state.clone(), after != before)
                    };
                    // Persist only on a durable-state transition; await it so a
                    // breach detected just before shutdown is not lost.
                    if changed {
                        if let Some(pg) = db_pool.as_ref() {
                            if let Err(e) = strategy::risk_monitor::RiskMonitor::persist_state(
                                pg, &risk_state,
                            )
                            .await
                            {
                                tracing::warn!(error = %e, "risk_state persist failed");
                            }
                        } else {
                            tracing::warn!(
                                "risk: db_pool unexpectedly None inside risk gate, skipping persist"
                            );
                        }
                    }

                    if halt_tick {
                        return;
                    }
                }

                // ── Operator pause gate (D-04) ─────────────────────────────────
                // Check operator_pause AFTER risk gate, BEFORE should_rebalance.
                // This is independent from IL-triggered pause_flag.
                if let Some(risk_arc) = &risk_monitor_opt {
                    let rm = risk_arc.lock().unwrap_or_else(|p| p.into_inner());
                    if rm.state.operator_pause {
                        tracing::debug!(pool = %pool_addr_clone, "operator pause active -- skipping rebalance");
                        return;
                    }
                }

                // ── Shadow rebalance decision (SHADOW-02) ───────────────────────
                // `should_rebalance` is infallible (returns `RebalanceDecision`),
                // so there is no error path to preserve here. The earlier
                // `Result<_, String>` wrapper was vestigial and made the
                // `error_flag=true` branch below unreachable. If a real fallible
                // decision path is ever introduced (e.g. catch_unwind around a
                // panicking strategy), reintroduce the Result here and route
                // errors into shadow_rebalances at that point.
                let rebalance_config = strategy::RebalanceConfig::default();
                let decision = strategy::should_rebalance(
                    pool.tick_current_index,
                    pos.tick_lower_index,
                    pos.tick_upper_index,
                    computed_fees_earned + computed_il_usd,
                    &rebalance_config,
                );

                let is_rebalance =
                    matches!(&decision, strategy::RebalanceDecision::Rebalance { .. });

                // Gate: if a rebalance plan were to be submitted, check shadow guard first.
                // In Phase 2 there is no real plan — we use the pool state as the proxy.
                // Real rebalance plan construction arrives in Phase 5.
                if is_rebalance {
                    // ── Telegram approval gate (TG-01, TG-02) ──────────────────
                    // When --telegram is active, send a proposal message and await
                    // /approve within the configured timeout before proceeding.
                    // We run on the processor task, so the (up to
                    // --approve-timeout-secs) wait never stalls the WS loop.
                    if let (Some(tg_bot), Some(tg_chat)) = (&telegram_bot, telegram_chat_id) {
                        // Build plan to get range_width for the proposal message.
                        let plan = execution::build_rebalance_plan(
                            &mint_str,
                            pool.tick_current_index,
                            pos.tick_lower_index,
                            pos.tick_upper_index,
                            pool.tick_spacing as i32,
                        );
                        let trigger_reason = match &decision {
                            strategy::RebalanceDecision::Rebalance { reason } => reason.clone(),
                            _ => "unknown".to_string(),
                        };
                        let proposal_data = bot::proposal::ProposalData {
                            pool_address: pool_addr_clone.clone(),
                            trigger_reason,
                            price: price_current,
                            simulated_fees_earned: computed_fees_earned,
                            simulated_il_usd: computed_il_usd,
                            simulated_net_pnl: computed_fees_earned - computed_il_usd.abs(),
                            range_width: (plan.new_tick_upper - plan.new_tick_lower) as f64,
                        };
                        let chat_id = teloxide::types::ChatId(tg_chat);

                        let (approved, skip_reason) = match bot::proposal::send_proposal(
                            tg_bot,
                            chat_id,
                            &proposal_data,
                            &pending_approval,
                        )
                        .await
                        {
                            Ok((proposal_id, rx)) => {
                                let approved = bot::proposal::await_approval(
                                    rx,
                                    approve_timeout_secs_val,
                                )
                                .await;
                                // Remove the dead sender on timeout so a
                                // later /approve cannot grab it and falsely
                                // report execution.
                                bot::proposal::clear_pending(&pending_approval, proposal_id);
                                (approved, "timeout")
                            }
                            Err(e) => {
                                // Fail closed: human approval is the only
                                // manual control before a live rebalance, so
                                // a Telegram failure must skip the rebalance,
                                // never silently authorize it.
                                tracing::warn!(
                                    error = %e,
                                    "failed to send Telegram proposal; \
                                     failing closed -- rebalance skipped"
                                );
                                (false, "telegram_error")
                            }
                        };

                        if !approved {
                            // Log skip to DB (TG-02) and continue to next tick.
                            if let Some(pg) = db_pool.as_ref() {
                                let _ = storage::writer::write_approval_skip(
                                    pg,
                                    &pool_addr_clone,
                                    skip_reason,
                                    price_current,
                                )
                                .await;
                            }
                            tracing::info!(
                                pool = %pool_addr_clone,
                                reason = skip_reason,
                                "rebalance skipped: approval not granted"
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
                // `decision` is infallible here (see block above), so no error
                // branch writes `error_flag=true`. If a fallible path is added,
                // restore the error arm that feeds the Plan 03 bad-row gate.
                if let Some(pg) = db_pool.as_ref() {
                    let shadow_row: Option<storage::writer::ShadowRebalanceRow> = match &decision {
                        strategy::RebalanceDecision::Rebalance { reason } => {
                            let plan = execution::build_rebalance_plan(
                                &mint_str,
                                pool.tick_current_index,
                                pos.tick_lower_index,
                                pos.tick_upper_index,
                                pool.tick_spacing as i32,
                            );
                            let range_width = (plan.new_tick_upper - plan.new_tick_lower) as f64;
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
                        strategy::RebalanceDecision::Hold { .. } => {
                            // No rebalance needed this tick — do not write a row.
                            None
                        }
                    };

                    if let Some(row) = shadow_row {
                        // Awaited (not fire-and-forget): a detached task would be
                        // cancelled by runtime drop on shutdown, losing a row the
                        // 14-day shadow gate depends on.
                        if let Err(e) = storage::writer::write_shadow_rebalance(pg, &row).await {
                            tracing::error!(
                                error = %e,
                                pool = %pool_addr_clone,
                                "failed to write shadow_rebalances row"
                            );
                        }
                    }
                }
                    }
                    .await;
                }
            });

            // The WS callback only enqueues; if the processor is saturated the
            // notification is dropped (the next one supersedes it anyway).
            let on_notify: data::ws::NotifyFn = Box::new(move |json: serde_json::Value| {
                if let Err(e) = tick_tx.try_send(json) {
                    tracing::warn!("tick queue full or closed; dropping pool update: {}", e);
                }
            });

            data::ws::watch_account(ws_url, pool_addr, shutdown_rx, on_notify).await;

            // watch_account dropped on_notify (and with it the channel sender),
            // so the processor exits once it drains the queue. Give in-flight
            // DB writes a bounded grace period before letting the runtime drop.
            match tokio::time::timeout(std::time::Duration::from_secs(5), processor).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => tracing::warn!("tick processor join error: {}", e),
                Err(_) => {
                    tracing::warn!("tick processor did not drain within 5s; continuing shutdown")
                }
            }

            // WR-03: graceful cleanup of background tasks before runtime drop.
            // `watch_account` returns only after the shutdown broadcast fires
            // (Ctrl+C handler) or the WS loop exits. The cex feed listens to
            // the same shutdown channel and will terminate by itself; we just
            // await its JoinHandle so it runs its disconnect path instead of
            // being killed by runtime shutdown. The telegram bot dispatcher
            // does not subscribe to the shutdown channel, so we abort() it.
            if let Some(h) = cex_handle {
                if let Err(e) = h.await {
                    tracing::warn!("cex_ws: join error on shutdown: {}", e);
                }
            }
            if let Some(h) = bot_handle {
                h.abort();
                let _ = h.await;
            }
        }
        Commands::Depth { pool } => {
            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let pool_data = rpc.fetch_account_checked(pool, &whirlpool_program)?;
            let pool_state = protocols::orca::parse_pool(&pool_data)?;

            let decimals_a = rpc.fetch_mint_decimals(&pool_state._token_mint_a)?;
            let decimals_b = rpc.fetch_mint_decimals(&pool_state._token_mint_b)?;
            // The impact math runs in the raw domain (raw price × raw
            // liquidity → costs in raw token-B units); convert for display:
            // prices ×10^(decA−decB), token-B costs ÷10^decB.
            let ui_factor = 10f64.powi(decimals_a as i32 - decimals_b as i32);
            let scale_b = 10f64.powi(decimals_b as i32);

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
                    pct,
                    buy.target_price * ui_factor,
                    buy.usd_needed / scale_b,
                    sell.usd_needed / scale_b
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

            let tick_deltas = extract_tick_deltas(&tick_arrays, pool_state.tick_spacing);

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
            // Histogram is display-only: convert bucket prices to UI units.
            let distribution_ui: Vec<analytics::depth::LiquidityLevel> = distribution
                .iter()
                .map(|l| analytics::depth::LiquidityLevel {
                    price: l.price * ui_factor,
                    liquidity: l.liquidity,
                })
                .collect();
            display::table::print_depth_histogram(&distribution_ui, price_current * ui_factor);
        }
        Commands::Impact { pool, size } => {
            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let pool_data = rpc.fetch_account_checked(pool, &whirlpool_program)?;
            let pool_state = protocols::orca::parse_pool(&pool_data)?;

            let decimals_a = rpc.fetch_mint_decimals(&pool_state._token_mint_a)?;
            let decimals_b = rpc.fetch_mint_decimals(&pool_state._token_mint_b)?;
            let ui_factor = 10f64.powi(decimals_a as i32 - decimals_b as i32);
            let scale_b = 10f64.powi(decimals_b as i32);

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
                    let tick_deltas = extract_tick_deltas(&tick_arrays, pool_state.tick_spacing);

                    // Build distribution around current tick (16 buckets each side).
                    let distribution = analytics::depth::build_distribution(
                        &tick_deltas,
                        pool_state.liquidity,
                        pool_state.tick_current_index,
                        pool_state.tick_spacing as i32,
                        16,
                    );

                    // Find the bucket containing the current tick (it is the middle bucket).
                    // Walk buckets in the buy direction (ascending price) consuming the
                    // trade size. The walk runs in the raw domain (raw prices × raw
                    // liquidity → costs in raw token-B units), so convert the human-USD
                    // --size to raw token-B once up front.
                    let mid = distribution.len() / 2;
                    let mut remaining = *size * scale_b;
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
                    // Fall back to constant-L approximation (raw domain: convert
                    // the human-USD size to raw token-B before dividing by the
                    // raw price).
                    tracing::warn!(
                        "tick-array fetch failed ({}); falling back to constant-L approximation",
                        e
                    );
                    let l = pool_state.liquidity as f64;
                    let sqrt_p = price_current.sqrt();
                    let amount_a = (*size * scale_b) / price_current;
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
            println!("Current price: ${:.6}", price_current * ui_factor);
            println!("Trade size:    ${:.0}", size);
            if impact_pct.is_finite() {
                println!("Price impact:  {:+.4}%", impact_pct);
                println!("Price after:   ${:.6}", target_price * ui_factor);
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
                let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
                // Same unit-correct P&L core as the `position` command.
                let p = compute_orca_pnl(&rpc, mint, entry_price)?;

                let config = strategy::RebalanceConfig {
                    rebalance_out_of_range: true,
                    near_edge_ticks: *near_edge_ticks,
                    min_net_pnl_usd: *min_pnl,
                };

                let decision = strategy::should_rebalance(
                    p.pool.tick_current_index,
                    p.pos.tick_lower_index,
                    p.pos.tick_upper_index,
                    p.net_pnl_usd,
                    &config,
                );

                println!("Position:     {}", p.position_pda);
                println!("Tick current: {}", p.pool.tick_current_index);
                println!(
                    "Range:        [{}, {}]",
                    p.pos.tick_lower_index, p.pos.tick_upper_index
                );
                println!("Net P&L:      ${:.2}", p.net_pnl_usd);
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
                println!("Migrations complete");
            }
        },
        Commands::Rebalance { mint, dry_run } => {
            if !*dry_run {
                anyhow::bail!("Only --dry-run is supported");
            }
            require_keypair_b58()?;

            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let (_position_pda, pos, pool) = load_orca_position_and_pool(&rpc, mint)?;

            let plan = execution::build_rebalance_plan(
                mint,
                pool.tick_current_index,
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
            decimals_a,
            decimals_b,
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
                    decimals_a: *decimals_a,
                    decimals_b: *decimals_b,
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
        Commands::Backfill {
            pool,
            from,
            to,
            timeframe,
            fee_bps,
            pool_liquidity,
            decimals_a,
            decimals_b,
            tick_spacing,
            dry_run,
        } => {
            use chrono::NaiveDate;

            let from_date: NaiveDate = from
                .parse()
                .map_err(|_| anyhow::anyhow!("--from must be a valid date (YYYY-MM-DD)"))?;
            let to_date: NaiveDate = to
                .parse()
                .map_err(|_| anyhow::anyhow!("--to must be a valid date (YYYY-MM-DD)"))?;
            if to_date <= from_date {
                anyhow::bail!("--to must be after --from");
            }
            if *pool_liquidity == 0 {
                anyhow::bail!(
                    "--pool-liquidity must be > 0 (fees accrue per unit of pool liquidity)"
                );
            }

            let from_ts = from_date
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp();
            let to_ts = to_date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

            let client = reqwest::Client::builder()
                .user_agent("tick-liq-backfill")
                .build()?;
            let candles =
                data::geckoterminal::fetch_range(&client, pool, timeframe, from_ts, to_ts).await?;
            if candles.is_empty() {
                anyhow::bail!(
                    "GeckoTerminal returned no candles for {pool} in [{from_date}, {to_date}). \
                     Check the pool address and timeframe ({timeframe})."
                );
            }

            let params = backtest::backfill::PoolSynthParams {
                pool_address: pool.clone(),
                fee_rate_bps: *fee_bps,
                pool_liquidity: *pool_liquidity,
                decimals_a: *decimals_a,
                decimals_b: *decimals_b,
                tick_spacing: *tick_spacing,
            };
            let ticks = backtest::backfill::synthesize_ticks(&candles, &params);

            let first = ticks.first().expect("non-empty: candles checked above");
            let last = ticks.last().expect("non-empty: candles checked above");
            let first_price =
                math::sqrt_price::sqrt_q64_to_ui_price(first.sqrt_price, *decimals_a, *decimals_b);
            let last_price =
                math::sqrt_price::sqrt_q64_to_ui_price(last.sqrt_price, *decimals_a, *decimals_b);
            let total_vol: f64 = candles.iter().map(|c| c.volume_usd).sum();

            println!(
                "Backfill — {} {} candles for pool {}",
                ticks.len(),
                timeframe,
                pool
            );
            println!(
                "Window:  {} → {}",
                first.observed_at.date_naive(),
                last.observed_at.date_naive()
            );
            println!("Price:   ${first_price:.4} → ${last_price:.4}");
            println!("Volume:  ${total_vol:.0} total");

            if *dry_run {
                println!("(dry-run — nothing written to the database)");
                return Ok(());
            }

            let db_url = cli.db_url.as_deref().ok_or_else(|| {
                anyhow::anyhow!("--db-url or DATABASE_URL is required (or use --dry-run)")
            })?;
            let pg = storage::connect(db_url).await?;
            for t in &ticks {
                storage::writer::write_pool_tick(&pg, t).await?;
            }
            println!(
                "Wrote {} pool_ticks rows. Replay with: \
                 backtest --pool {pool} --from {from} --to {to} \
                 --position-liquidity <L> --fee-bps {fee_bps} \
                 --decimals-a {decimals_a} --decimals-b {decimals_b}",
                ticks.len()
            );
        }
        Commands::Hedge { mint, dry_run } => {
            if !*dry_run {
                anyhow::bail!("Only --dry-run is supported");
            }
            require_keypair_b58()?;

            let rpc = rpc::SolanaRpc::with_timeout(&cli.rpc_url, cli.rpc_timeout);
            let (_position_pda, pos, pool) = load_orca_position_and_pool(&rpc, mint)?;

            // The LP's delta-hedgeable exposure is its token-A leg: the
            // position holds `amount_a` of the volatile token, so the
            // offsetting perp notional is amount_a (UI units) × UI price.
            // (Raw greeks delta × raw price is off by orders of magnitude —
            // wrong units on both factors.)
            let decimals_a = rpc.fetch_mint_decimals(&pool._token_mint_a)?;
            let decimals_b = rpc.fetch_mint_decimals(&pool._token_mint_b)?;
            let price_current =
                analytics::greeks::sqrt_q64_to_ui_price(pool.sqrt_price, decimals_a, decimals_b);
            let amounts = analytics::amounts::compute_token_amounts(
                pos.liquidity,
                pool.sqrt_price,
                pos.tick_lower_index,
                pos.tick_upper_index,
            )?;
            let exposure_a_ui = amounts.amount_a as f64 / 10f64.powi(decimals_a as i32);

            // delta > 0 (long token A) → short perp, per compute_hedge_size.
            let mut plan = execution::compute_hedge_size(exposure_a_ui, price_current);
            plan.position_mint = mint.clone();
            execution::print_hedge_dry_run(&plan);
        }
    }

    Ok(())
}
