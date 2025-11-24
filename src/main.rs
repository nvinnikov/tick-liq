use anyhow::Result;
use clap::{Parser, Subcommand};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

mod analytics;
mod data;
mod display;
mod protocols;
mod rpc;

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
    },
    /// Watch a position in real-time
    Watch {
        /// Position NFT mint address
        mint: String,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match &cli.command {
        Commands::Position { mint, protocol } => {
            let rpc = rpc::SolanaRpc::new(&cli.rpc_url);

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

                    let fees_usd = (pos.fee_owed_a + accrued_a) as f64 / 1e9 * price_current
                        + (pos.fee_owed_b + accrued_b) as f64 / 1e6;

                    let il_fraction =
                        analytics::pnl::compute_il(0.0, price_current, price_lower, price_upper);
                    let position_value = amounts.amount_a as f64 / 1e9 * price_current
                        + amounts.amount_b as f64 / 1e6;
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
                        decimals_a: 9,
                        decimals_b: 6,
                        symbol_a: "A".to_string(),
                        symbol_b: "B".to_string(),
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

                    let price_current = analytics::greeks::sqrt_q64_to_price(pool.sqrt_price_x64);

                    println!("Raydium Position: {}", position_pda);
                    println!("Pool:     {}", pos.pool_id);
                    println!("Price:    ${:.4}", price_current);
                    println!(
                        "Tick:     {} (range: {} -- {})",
                        pool.tick_current, pos.tick_lower_index, pos.tick_upper_index
                    );
                    println!("Liquidity: {}", pos.liquidity);
                }
                other => anyhow::bail!("Unknown protocol '{}'. Use 'orca' or 'raydium'.", other),
            }
        }
        Commands::Watch { mint } => {
            let rpc = rpc::SolanaRpc::new(&cli.rpc_url);
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

            let ws_url = cli
                .rpc_url
                .replace("https://", "wss://")
                .replace("http://", "ws://");

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
            let on_notify = Box::new(move |_json: serde_json::Value| {
                let rpc_inner = rpc::SolanaRpc::new(&rpc_url);
                print!("\x1B[2J\x1B[1;1H");
                println!(
                    "[{}] Pool update received",
                    chrono::Utc::now().format("%H:%M:%S UTC")
                );
                println!();

                let pool_data = match rpc_inner
                    .fetch_account_checked(&pool_addr_clone, &whirlpool_program)
                {
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

                let price_current = analytics::greeks::sqrt_q64_to_price(pool.sqrt_price);
                let in_range = pool.tick_current_index >= pos.tick_lower_index
                    && pool.tick_current_index <= pos.tick_upper_index;

                println!("Pool:      {}", pool_addr_clone);
                println!("Price:     ${:.4}", price_current);
                println!("Tick:      {}", pool.tick_current_index);
                println!("In range:  {}", if in_range { "YES" } else { "NO -- needs rebalance" });
                println!("Liquidity: {}", pool.liquidity);
            });

            data::ws::watch_account(ws_url, pool_addr, shutdown_rx, on_notify).await;
        }
        Commands::Depth { pool } => {
            let rpc = rpc::SolanaRpc::new(&cli.rpc_url);
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

            println!();
            println!(
                "Note: uses pool-level liquidity. Tick-array depth map coming in a future update."
            );
        }
        Commands::Impact { pool, size } => {
            let rpc = rpc::SolanaRpc::new(&cli.rpc_url);
            let whirlpool_program = protocols::orca::whirlpool_program_pubkey();
            let pool_data = rpc.fetch_account_checked(pool, &whirlpool_program)?;
            let pool_state = protocols::orca::parse_pool(&pool_data)?;

            let price_current = analytics::greeks::sqrt_q64_to_price(pool_state.sqrt_price);

            let l = pool_state.liquidity as f64;
            let sqrt_p = price_current.sqrt();
            let amount_a = size / price_current;
            let inv_sqrt_target = (1.0 / sqrt_p) - (amount_a / l);

            let (target_price, impact_pct) = if inv_sqrt_target > 0.0 {
                let p_target = 1.0 / (inv_sqrt_target * inv_sqrt_target);
                let pct = (p_target - price_current) / price_current * 100.0;
                (p_target, pct)
            } else {
                (f64::INFINITY, f64::INFINITY)
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
    }

    Ok(())
}
