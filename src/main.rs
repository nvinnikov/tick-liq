use anyhow::Result;
use clap::{Parser, Subcommand};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

mod rpc;
mod protocols;
mod analytics;
mod display;

#[derive(Parser)]
#[command(name = "lp-inspect")]
#[command(about = "CLMM position inspector for Solana")]
struct Cli {
    #[arg(long, env = "SOLANA_RPC_URL", default_value = "https://api.devnet.solana.com")]
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
                    let whirlpool_program =
                        Pubkey::from_str(protocols::orca::WHIRLPOOL_PROGRAM_ID)?;
                    let mint_pubkey = Pubkey::from_str(mint)?;

                    // Orca position PDA: seeds = ["position", position_mint]
                    let (position_pda, _) = Pubkey::find_program_address(
                        &[b"position", mint_pubkey.as_ref()],
                        &whirlpool_program,
                    );

                    let position_data = rpc.fetch_account_data(&position_pda.to_string())?;
                    let position = protocols::orca::parse_position(&position_data)?;

                    println!("Position PDA:  {}", position_pda);
                    println!("Pool:          {}", position.whirlpool);
                    println!("Liquidity:     {}", position.liquidity);
                    println!("Tick lower:    {}", position.tick_lower_index);
                    println!("Tick upper:    {}", position.tick_upper_index);
                    println!("Fee owed A:    {}", position.fee_owed_a);
                    println!("Fee owed B:    {}", position.fee_owed_b);

                    let pool_data = rpc.fetch_account_data(&position.whirlpool.to_string())?;
                    let pool = protocols::orca::parse_pool(&pool_data)?;

                    println!("Current tick:  {}", pool.tick_current_index);
                    println!("Sqrt price:    {}", pool.sqrt_price);
                    println!("Fee rate:      {:.2} bps", pool.fee_rate as f64 / 100.0);
                }
                other => anyhow::bail!("Unknown protocol '{}'. Use 'orca' or 'raydium'.", other),
            }
        }
        Commands::Watch { mint } => {
            println!("TODO: watch {}", mint);
        }
        Commands::Depth { pool } => {
            println!("TODO: depth {}", pool);
        }
        Commands::Impact { pool, size } => {
            println!("TODO: impact {} size={}", pool, size);
        }
    }

    Ok(())
}