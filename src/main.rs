use anyhow::Result;
use clap::{Parser, Subcommand};

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
            println!("TODO: position {} ({})", mint, protocol);
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