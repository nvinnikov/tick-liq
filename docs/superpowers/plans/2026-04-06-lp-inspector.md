# LP Inspector CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `lp-inspect`, a CLI that prints real-time P&L, token amounts, Greeks, and liquidity depth for CLMM positions on Solana (Orca + Raydium).

**Architecture:** Single binary with four layers: `rpc` (fetch Solana accounts), `protocols` (deserialize account bytes), `analytics` (compute metrics), `display` (format terminal output). Layers are one-way: each layer only knows about the layers above it, not below.

**Tech Stack:** Rust 2021, tokio (async), clap v4 (CLI), solana-client 1.18, orca_whirlpools_core, borsh 0.10, anyhow, tokio-tungstenite

---

## File Map

| File | Responsibility |
|------|----------------|
| `Cargo.toml` | Dependencies, binary definition |
| `src/main.rs` | CLI definition (clap), command routing |
| `src/rpc.rs` | Fetch raw account bytes from Solana RPC |
| `src/protocols/mod.rs` | Re-export protocol modules |
| `src/protocols/orca.rs` | Whirlpool pool + position deserialization |
| `src/protocols/raydium.rs` | Raydium CLMM pool + position deserialization |
| `src/analytics/mod.rs` | Re-export analytics modules, shared types |
| `src/analytics/amounts.rs` | Token amounts from liquidity using orca_whirlpools_core |
| `src/analytics/pnl.rs` | Fees earned, IL, net P&L |
| `src/analytics/greeks.rs` | Position delta and gamma |
| `src/analytics/depth.rs` | Liquidity distribution, price impact |
| `src/display/mod.rs` | Re-export display modules |
| `src/display/table.rs` | Formatted terminal output |

---

## Task 1: Project setup

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

- [ ] **Step 1: Verify Orca math crate name**

Run: `cargo search orca_whirlpools_core`

Expected: version listed on crates.io. Note the latest version. If not found, run `cargo search whirlpool` and look for Orca's official crate.

- [ ] **Step 2: Write Cargo.toml**

```toml
[package]
name = "tick-liq"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "lp-inspect"
path = "src/main.rs"

[dependencies]
# Solana
solana-client = "1.18"
solana-sdk = "1.18"

# Orca math — verify version from Step 1
orca_whirlpools_core = "1"

# Serialization
borsh = "0.10"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Async
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = "0.21"
futures-util = "0.3"

# CLI
clap = { version = "4", features = ["derive"] }

# Error handling
anyhow = "1"

# Time (for watch command)
chrono = "0.4"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

- [ ] **Step 3: Write src/main.rs**

```rust
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
    tracing_subscriber::init();
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
```

Create stub files to satisfy `mod` declarations:

`src/rpc.rs`: `// stub`
`src/protocols/mod.rs`: `// stub`
`src/analytics/mod.rs`: `// stub`
`src/display/mod.rs`: `// stub`

- [ ] **Step 4: Build**

Run: `cargo build`

Expected: compiles. If `orca_whirlpools_core` not found, correct the crate name in Cargo.toml.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/
git commit -m "feat: project setup with CLI skeleton"
```

---

## Task 2: RPC client

**Files:**
- Modify: `src/rpc.rs`

- [ ] **Step 1: Write the test first**

Replace `src/rpc.rs` entirely:

```rust
use anyhow::{anyhow, Result};
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

pub struct SolanaRpc {
    pub client: RpcClient,
}

impl SolanaRpc {
    pub fn new(url: &str) -> Self {
        Self {
            client: RpcClient::new(url.to_string()),
        }
    }

    /// Fetch raw account bytes. Returns error if account not found.
    pub fn fetch_account_data(&self, address: &str) -> Result<Vec<u8>> {
        let pubkey = Pubkey::from_str(address)
            .map_err(|e| anyhow!("Invalid address '{}': {}", address, e))?;

        let account = self.client
            .get_account(&pubkey)
            .map_err(|e| anyhow!("Account '{}' not found: {}", address, e))?;

        Ok(account.data)
    }

    /// Fetch account program owner. Used to verify account type before deserializing.
    pub fn fetch_owner(&self, address: &str) -> Result<Pubkey> {
        let pubkey = Pubkey::from_str(address)
            .map_err(|e| anyhow!("Invalid address '{}': {}", address, e))?;

        let account = self.client
            .get_account(&pubkey)
            .map_err(|e| anyhow!("Account '{}' not found: {}", address, e))?;

        Ok(account.owner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_address_returns_error() {
        let rpc = SolanaRpc::new("https://api.devnet.solana.com");
        let result = rpc.fetch_account_data("not_a_valid_pubkey");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid address"));
    }
}
```

- [ ] **Step 2: Run test**

Run: `cargo test rpc`

Expected: `test rpc::tests::test_invalid_address_returns_error ... ok`

- [ ] **Step 3: Commit**

```bash
git add src/rpc.rs
git commit -m "feat: Solana RPC client with account fetching"
```

---

## Task 3: Orca account deserialization

**Files:**
- Modify: `src/protocols/mod.rs`
- Create: `src/protocols/orca.rs`

**Background (Rust concept):** Solana accounts are raw bytes. Anchor programs prefix every account with an 8-byte discriminator. We skip those first 8 bytes, then use `borsh` to decode the rest into a typed struct. Borsh decodes fields in declaration order — the struct must match the on-chain layout exactly.

- [ ] **Step 1: Replace src/protocols/mod.rs**

```rust
pub mod orca;
```

- [ ] **Step 2: Write failing test and implementation in src/protocols/orca.rs**

```rust
use anyhow::{anyhow, Result};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;

pub const WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

/// First 8 bytes of every Anchor account are a discriminator — skip them.
const DISC: usize = 8;

/// Key fields of an Orca Whirlpool pool account.
/// Field order matches the on-chain Anchor struct exactly.
/// Reference: https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/state/whirlpool.rs
#[derive(BorshDeserialize, Debug, Clone)]
pub struct WhirlpoolPool {
    pub whirlpools_config: Pubkey,
    pub whirlpool_bump: [u8; 1],
    pub tick_spacing: u16,
    pub tick_spacing_seed: [u8; 2],
    pub fee_rate: u16,           // hundredths of a bip; 300 = 0.03%
    pub protocol_fee_rate: u16,
    pub liquidity: u128,
    pub sqrt_price: u128,        // Q64.64 fixed-point
    pub tick_current_index: i32,
    pub protocol_fee_owed_a: u64,
    pub protocol_fee_owed_b: u64,
    pub token_mint_a: Pubkey,
    pub token_vault_a: Pubkey,
    pub fee_growth_global_a: u128,
    pub token_mint_b: Pubkey,
    pub token_vault_b: Pubkey,
    pub fee_growth_global_b: u128,
    pub reward_last_updated_timestamp: u64,
    // reward_infos (3 × 128 bytes) omitted — not needed for analytics
}

/// Key fields of an Orca Whirlpool position account.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct WhirlpoolPosition {
    pub whirlpool: Pubkey,
    pub position_mint: Pubkey,
    pub liquidity: u128,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub fee_growth_checkpoint_a: u128,
    pub fee_owed_a: u64,
    pub fee_growth_checkpoint_b: u128,
    pub fee_owed_b: u64,
    // reward_infos omitted
}

pub fn parse_pool(data: &[u8]) -> Result<WhirlpoolPool> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    WhirlpoolPool::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Whirlpool pool: {}", e))
}

pub fn parse_position(data: &[u8]) -> Result<WhirlpoolPosition> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    WhirlpoolPosition::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Whirlpool position: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pool_too_short_returns_error() {
        let result = parse_pool(&[0u8; 4]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn test_parse_position_too_short_returns_error() {
        let result = parse_position(&[0u8; 4]);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test protocols::orca`

Expected: both tests pass.

- [ ] **Step 4: Build**

Run: `cargo build`

Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add src/protocols/
git commit -m "feat: Orca Whirlpool account deserialization"
```

---

## Task 4: Basic `position` command — fetch and print raw data

Wire up the `position` command to fetch a real position from devnet. No math yet — confirm the data pipeline works end-to-end.

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace Commands::Position arm in main()**

Add `use std::str::FromStr;` at the top of `src/main.rs`.

Replace the `Commands::Position` arm:

```rust
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
```

Add `use solana_sdk::pubkey::Pubkey;` and `use std::str::FromStr;` at the top of `src/main.rs`.

- [ ] **Step 2: Build**

Run: `cargo build`

Expected: compiles.

- [ ] **Step 3: Test against a real devnet position**

Create a test position: go to https://app.orca.so (devnet mode), open a position, copy the position NFT mint address. Then run:

```bash
SOLANA_RPC_URL=https://api.devnet.solana.com cargo run -- position <POSITION_MINT>
```

Expected: prints tick range, liquidity, fee rate without errors.

If borsh deserialization fails with "Failed to deserialize", the struct field order is wrong. Compare field layout with https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/state/whirlpool.rs and update `WhirlpoolPool` / `WhirlpoolPosition` struct fields.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire position command to fetch raw Orca position data"
```

---

## Task 5: Token amounts from liquidity

**Files:**
- Modify: `src/analytics/mod.rs`
- Create: `src/analytics/amounts.rs`

**Background:** The amounts of token A and B in a CLMM position depend on where the current price sits relative to the range. `orca_whirlpools_core` has a validated function for this.

- [ ] **Step 1: Check orca_whirlpools_core function names**

Run: `cargo doc --open`

Navigate to `orca_whirlpools_core` and find:
- A function that converts tick index to sqrt_price
- A function that computes token amounts from liquidity + sqrt prices

The functions are likely named `tick_index_to_sqrt_price` and `get_token_amounts_from_liquidity`. Note exact names and signatures — use them in Step 3.

- [ ] **Step 2: Replace src/analytics/mod.rs**

```rust
pub mod amounts;
```

- [ ] **Step 3: Write failing tests, then implementation in src/analytics/amounts.rs**

```rust
use anyhow::Result;
use orca_whirlpools_core::{get_token_amounts_from_liquidity, tick_index_to_sqrt_price};

/// Token amounts held in a position, in raw on-chain units (before decimal adjustment).
#[derive(Debug, Clone, PartialEq)]
pub struct TokenAmounts {
    pub amount_a: u64,
    pub amount_b: u64,
}

/// Compute token amounts for a position.
///
/// - liquidity: position's liquidity (u128 from account)
/// - sqrt_price: pool's current sqrt_price in Q64.64 (u128 from account)
/// - tick_lower / tick_upper: position range bounds
pub fn compute_token_amounts(
    liquidity: u128,
    sqrt_price: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> Result<TokenAmounts> {
    let sqrt_lower = tick_index_to_sqrt_price(tick_lower);
    let sqrt_upper = tick_index_to_sqrt_price(tick_upper);

    let (amount_a, amount_b) = get_token_amounts_from_liquidity(
        liquidity,
        sqrt_price,
        sqrt_lower,
        sqrt_upper,
        false, // round_down (conservative)
    );

    Ok(TokenAmounts { amount_a, amount_b })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqrt_price_at_tick(tick: i32) -> u128 {
        tick_index_to_sqrt_price(tick)
    }

    #[test]
    fn test_price_below_range_all_token_a_no_token_b() {
        // Price below range: all liquidity is token A
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_at_tick(50),  // current below range [100, 200]
            100,
            200,
        ).unwrap();
        assert!(amounts.amount_a > 0, "token A should be > 0 below range");
        assert_eq!(amounts.amount_b, 0, "token B should be 0 below range");
    }

    #[test]
    fn test_price_above_range_all_token_b_no_token_a() {
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_at_tick(300), // current above range [100, 200]
            100,
            200,
        ).unwrap();
        assert_eq!(amounts.amount_a, 0, "token A should be 0 above range");
        assert!(amounts.amount_b > 0, "token B should be > 0 above range");
    }

    #[test]
    fn test_price_in_range_has_both_tokens() {
        let amounts = compute_token_amounts(
            1_000_000,
            sqrt_price_at_tick(150), // in range [100, 200]
            100,
            200,
        ).unwrap();
        assert!(amounts.amount_a > 0, "token A should be > 0 in range");
        assert!(amounts.amount_b > 0, "token B should be > 0 in range");
    }

    #[test]
    fn test_zero_liquidity_returns_zero_amounts() {
        let amounts = compute_token_amounts(
            0,
            sqrt_price_at_tick(150),
            100,
            200,
        ).unwrap();
        assert_eq!(amounts.amount_a, 0);
        assert_eq!(amounts.amount_b, 0);
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test analytics::amounts`

Expected: all 4 tests pass.

If `tick_index_to_sqrt_price` or `get_token_amounts_from_liquidity` are not found, open the crate docs (`cargo doc --open`) and find the correct function names. Update imports and calls in this file.

- [ ] **Step 5: Build**

Run: `cargo build`

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src/analytics/
git commit -m "feat: compute token amounts from CLMM liquidity using orca_whirlpools_core"
```

---

## Task 6: P&L — fees and impermanent loss

**Files:**
- Modify: `src/analytics/mod.rs` (add `pub mod pnl;`)
- Create: `src/analytics/pnl.rs`

**Background:**
- **Fees earned:** The position account stores `fee_owed_a` and `fee_owed_b` — the accumulated fees not yet collected. We also compute fees accrued since the last checkpoint using `(fee_growth_global - fee_growth_checkpoint) * liquidity / 2^128`.
- **IL:** `IL = 2*sqrt(P_current/P_entry) / (1 + P_current/P_entry) - 1`. This is the standard concentrated liquidity IL formula. Always ≤ 0. Requires knowing the entry price (stored externally; in this first version we accept it as a parameter).

- [ ] **Step 1: Add pnl to analytics/mod.rs**

```rust
pub mod amounts;
pub mod pnl;
```

- [ ] **Step 2: Write failing tests, then implementation in src/analytics/pnl.rs**

```rust
/// P&L result in USD (token B equivalent).
#[derive(Debug, Clone)]
pub struct PnlResult {
    pub fees_usd: f64,
    pub il_usd: f64,       // always <= 0
    pub net_usd: f64,
    pub initial_value_usd: f64,
}

impl PnlResult {
    pub fn fees_pct(&self) -> f64 {
        if self.initial_value_usd == 0.0 { return 0.0; }
        self.fees_usd / self.initial_value_usd * 100.0
    }

    pub fn il_pct(&self) -> f64 {
        if self.initial_value_usd == 0.0 { return 0.0; }
        self.il_usd / self.initial_value_usd * 100.0
    }

    pub fn net_pct(&self) -> f64 {
        if self.initial_value_usd == 0.0 { return 0.0; }
        self.net_usd / self.initial_value_usd * 100.0
    }
}

/// Compute impermanent loss as a fraction (e.g. -0.02 = -2%).
///
/// Uses the standard concentrated liquidity IL formula.
/// Clamps prices to range boundaries before computing.
/// Returns 0.0 if entry price is 0 (unknown).
pub fn compute_il(
    price_entry: f64,
    price_current: f64,
    price_lower: f64,
    price_upper: f64,
) -> f64 {
    if price_entry == 0.0 { return 0.0; }

    let pa = price_lower.sqrt();
    let pb = price_upper.sqrt();
    let sp0 = price_entry.sqrt().clamp(pa, pb);
    let sp1 = price_current.sqrt().clamp(pa, pb);

    let ratio = sp1 / sp0;
    // V_lp / V_hodl = 2*sqrt(ratio) / (1 + ratio)
    let lp_relative = 2.0 * ratio.sqrt() / (1.0 + ratio);

    lp_relative - 1.0  // always <= 0
}

/// Compute fees accrued since last on-chain checkpoint (not yet in fee_owed).
///
/// Orca accumulates fees as: fee_growth_per_unit_liquidity * 2^128.
/// Uncollected = (fee_growth_global - fee_growth_checkpoint) * liquidity / 2^128.
pub fn compute_accrued_fees(
    fee_growth_global: u128,
    fee_growth_checkpoint: u128,
    liquidity: u128,
) -> u64 {
    let growth_delta = fee_growth_global.wrapping_sub(fee_growth_checkpoint);
    // (growth_delta * liquidity) >> 128, computed without overflow via u128 halving
    let hi = (growth_delta >> 64) * (liquidity >> 64);
    let lo_hi = (growth_delta & u64::MAX as u128) * (liquidity >> 64);
    let hi_lo = (growth_delta >> 64) * (liquidity & u64::MAX as u128);
    let result = hi + (lo_hi >> 64) + (hi_lo >> 64);
    result as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_il_zero_at_entry_price() {
        let il = compute_il(100.0, 100.0, 80.0, 120.0);
        assert!(il.abs() < 1e-10, "IL at entry price should be ~0, got {}", il);
    }

    #[test]
    fn test_il_always_non_positive() {
        for price in [50.0, 80.0, 90.0, 100.0, 110.0, 130.0, 200.0] {
            let il = compute_il(100.0, price, 80.0, 120.0);
            assert!(il <= 0.0, "IL must be <= 0 for price {}, got {}", price, il);
        }
    }

    #[test]
    fn test_il_zero_when_entry_unknown() {
        assert_eq!(compute_il(0.0, 150.0, 80.0, 120.0), 0.0);
    }

    #[test]
    fn test_accrued_fees_zero_when_growth_unchanged() {
        assert_eq!(compute_accrued_fees(1000, 1000, 1_000_000), 0);
    }

    #[test]
    fn test_accrued_fees_increase_with_growth_delta() {
        let small = compute_accrued_fees(1_001, 1_000, 1_000_000);
        let large = compute_accrued_fees(1_010, 1_000, 1_000_000);
        assert!(large > small);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test analytics::pnl`

Expected: all 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/analytics/pnl.rs src/analytics/mod.rs
git commit -m "feat: compute IL and accrued fees for CLMM position"
```

---

## Task 7: Greeks (delta, gamma)

**Files:**
- Modify: `src/analytics/mod.rs` (add `pub mod greeks;`)
- Create: `src/analytics/greeks.rs`

**Background:** LP delta is how much position value changes per $1 price increase. When in range, LP is naturally short vol: delta = `-L / (2 * sqrt(P) * P)`. Outside range, delta = 0 (position is fully in one token, no sensitivity to small moves). Gamma = rate of change of delta.

- [ ] **Step 1: Add greeks to analytics/mod.rs**

```rust
pub mod amounts;
pub mod pnl;
pub mod greeks;
```

- [ ] **Step 2: Write failing tests and implementation in src/analytics/greeks.rs**

```rust
#[derive(Debug, Clone)]
pub struct Greeks {
    /// Rate of change of position value per $1 price increase.
    /// Negative when in range (LP is short volatility).
    pub delta: f64,
    /// Rate of change of delta per $1 price increase.
    pub gamma: f64,
}

/// Convert Q64.64 sqrt_price to f64 price.
pub fn sqrt_price_q64_to_price(sqrt_price_q64: u128) -> f64 {
    let sqrt_p = sqrt_price_q64 as f64 / (1u128 << 64) as f64;
    sqrt_p * sqrt_p
}

/// Compute position Greeks.
///
/// Returns delta=0, gamma=0 when price is outside the range.
pub fn compute_greeks(
    liquidity: u128,
    sqrt_price_q64: u128,
    tick_lower: i32,
    tick_upper: i32,
) -> Greeks {
    let sqrt_p = (sqrt_price_q64 as f64) / (1u128 << 64) as f64;
    let price = sqrt_p * sqrt_p;

    let price_lower = 1.0001f64.powi(tick_lower);
    let price_upper = 1.0001f64.powi(tick_upper);

    if price < price_lower || price > price_upper {
        return Greeks { delta: 0.0, gamma: 0.0 };
    }

    let l = liquidity as f64;

    // delta = -L / (2 * sqrt(P) * P)  [from CLAUDE.md]
    let delta = -l / (2.0 * sqrt_p * price);

    // gamma = L / (2 * P^(5/2))
    let gamma = l / (2.0 * price * price * sqrt_p);

    Greeks { delta, gamma }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q64_at_tick(tick: i32) -> u128 {
        let sqrt_p = (1.0001f64.powi(tick)).sqrt();
        (sqrt_p * (1u128 << 64) as f64) as u128
    }

    #[test]
    fn test_delta_negative_when_in_range() {
        let greeks = compute_greeks(1_000_000, q64_at_tick(0), -100, 100);
        assert!(greeks.delta < 0.0, "delta should be negative in range");
    }

    #[test]
    fn test_gamma_positive_when_in_range() {
        let greeks = compute_greeks(1_000_000, q64_at_tick(0), -100, 100);
        assert!(greeks.gamma > 0.0, "gamma should be positive in range");
    }

    #[test]
    fn test_delta_zero_above_range() {
        let greeks = compute_greeks(1_000_000, q64_at_tick(200), -100, 100);
        assert_eq!(greeks.delta, 0.0);
    }

    #[test]
    fn test_larger_liquidity_larger_abs_delta() {
        let small = compute_greeks(100, q64_at_tick(0), -100, 100);
        let large = compute_greeks(1_000_000, q64_at_tick(0), -100, 100);
        assert!(large.delta.abs() > small.delta.abs());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test analytics::greeks`

Expected: all 4 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/analytics/greeks.rs src/analytics/mod.rs
git commit -m "feat: compute LP position delta and gamma"
```

---

## Task 8: Full formatted position output

**Files:**
- Modify: `src/display/mod.rs`
- Create: `src/display/table.rs`
- Modify: `src/main.rs` (replace TODO with real output)

- [ ] **Step 1: Replace src/display/mod.rs**

```rust
pub mod table;
```

- [ ] **Step 2: Write failing test and implementation in src/display/table.rs**

```rust
use crate::analytics::amounts::TokenAmounts;
use crate::analytics::greeks::Greeks;
use crate::analytics::pnl::PnlResult;

// Uses owned Strings to avoid lifetime complexity (no &'a str).
pub struct PositionSummary {
    pub pool_address: String,
    pub fee_rate_bps: f64,
    pub price_lower: f64,
    pub price_upper: f64,
    pub price_current: f64,
    pub in_range: bool,
    pub range_pct: f64,          // 0–100, position within the range
    pub amounts: TokenAmounts,
    pub decimals_a: u8,
    pub decimals_b: u8,
    pub symbol_a: String,
    pub symbol_b: String,
    pub pnl: PnlResult,
    pub greeks: Greeks,
}

pub fn print_position(s: &PositionSummary) {
    let label = format!(
        "Position: {}...  (Orca {:.2} bps)",
        &s.pool_address[..8],
        s.fee_rate_bps
    );
    let sep = "─".repeat(label.len());

    println!("{}", label);
    println!("{}", sep);

    let status = if s.in_range {
        format!("✓ IN RANGE  ({:.0}%)", s.range_pct)
    } else {
        "✗ OUT OF RANGE".to_string()
    };

    println!("Range:      ${:.4} — ${:.4}", s.price_lower, s.price_upper);
    println!("Current:    ${:.4}  {}", s.price_current, status);
    println!();

    let a = s.amounts.amount_a as f64 / 10f64.powi(s.decimals_a as i32);
    let b = s.amounts.amount_b as f64 / 10f64.powi(s.decimals_b as i32);
    println!("Amounts:    {:.6} {}  +  {:.2} {}", a, s.symbol_a, b, s.symbol_b);
    println!();

    println!("P&L:");
    println!("  Fees:  {:+.2}  ({:+.2}%)", s.pnl.fees_usd, s.pnl.fees_pct());
    println!("  IL:    {:+.2}  ({:+.2}%)", s.pnl.il_usd, s.pnl.il_pct());
    println!("  Net:   {:+.2}  ({:+.2}%)", s.pnl.net_usd, s.pnl.net_pct());
    println!();

    println!("Delta: {:.4}   Gamma: {:.6}", s.greeks.delta, s.greeks.gamma);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::amounts::TokenAmounts;
    use crate::analytics::greeks::Greeks;
    use crate::analytics::pnl::PnlResult;

    #[test]
    fn test_print_position_does_not_panic() {
        let amounts = TokenAmounts { amount_a: 1_000_000_000, amount_b: 150_000_000 };
        let pnl = PnlResult {
            fees_usd: 10.0,
            il_usd: -3.0,
            net_usd: 7.0,
            initial_value_usd: 1000.0,
        };
        let greeks = Greeks { delta: -0.34, gamma: 0.02 };

        let s = PositionSummary {
            pool_address: "11111111111111111111111111111111".to_string(),
            fee_rate_bps: 30.0,
            price_lower: 100.0,
            price_upper: 200.0,
            price_current: 150.0,
            in_range: true,
            range_pct: 50.0,
            amounts,
            decimals_a: 9,
            decimals_b: 6,
            symbol_a: "SOL".to_string(),
            symbol_b: "USDC".to_string(),
            pnl,
            greeks,
        };

        print_position(&s); // should not panic
    }
}
```

- [ ] **Step 3: Run test**

Run: `cargo test display`

Expected: test passes (output printed to stdout is fine in tests).

- [ ] **Step 4: Wire full position output in main.rs**

Replace the `"orca"` arm in `Commands::Position`:

```rust
"orca" => {
    use orca_whirlpools_core::tick_index_to_sqrt_price;

    let rpc = rpc::SolanaRpc::new(&cli.rpc_url);
    let whirlpool_program = Pubkey::from_str(protocols::orca::WHIRLPOOL_PROGRAM_ID)?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let (position_pda, _) = Pubkey::find_program_address(
        &[b"position", mint_pubkey.as_ref()],
        &whirlpool_program,
    );

    let position_data = rpc.fetch_account_data(&position_pda.to_string())?;
    let pos = protocols::orca::parse_position(&position_data)?;

    let pool_data = rpc.fetch_account_data(&pos.whirlpool.to_string())?;
    let pool = protocols::orca::parse_pool(&pool_data)?;

    // sqrt_price Q64.64 → f64 price
    let to_f64_price = |sqrt_q64: u128| -> f64 {
        let s = sqrt_q64 as f64 / (1u128 << 64) as f64;
        s * s
    };

    let price_current = to_f64_price(pool.sqrt_price);
    let price_lower = to_f64_price(tick_index_to_sqrt_price(pos.tick_lower_index));
    let price_upper = to_f64_price(tick_index_to_sqrt_price(pos.tick_upper_index));

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

    // Fees: use on-chain fee_owed (fees ready to collect)
    // Accrued fees since last checkpoint (approximate)
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

    // Convert to USD (token B = stable, token A * current price)
    let fees_usd = (pos.fee_owed_a + accrued_a) as f64 / 1e9 * price_current
        + (pos.fee_owed_b + accrued_b) as f64 / 1e6;

    // IL: requires entry price. Pass 0.0 when unknown — IL shows as 0.
    // Future phase: store entry price when position is opened.
    let il_fraction = analytics::pnl::compute_il(0.0, price_current, price_lower, price_upper);
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
        decimals_a: 9,  // fetch from token mint account in a future task
        decimals_b: 6,
        symbol_a: "A".to_string(),  // fetch from token metadata in a future task
        symbol_b: "B".to_string(),
        pnl,
        greeks,
    };

    display::table::print_position(&summary);
}
```

- [ ] **Step 5: Build**

Run: `cargo build`

Expected: compiles.

- [ ] **Step 6: Test end-to-end on devnet**

```bash
cargo run -- position <DEVNET_POSITION_MINT>
```

Expected: formatted table with range, amounts, P&L, Greeks printed. Symbols show "A"/"B" (real names added in a later task). IL shows 0.0 (entry price not tracked yet — expected).

- [ ] **Step 7: Commit**

```bash
git add src/display/ src/main.rs
git commit -m "feat: full formatted position output with amounts, P&L, and Greeks"
```

---

## Task 9: Liquidity depth and price impact

**Files:**
- Modify: `src/analytics/mod.rs` (add `pub mod depth;`)
- Create: `src/analytics/depth.rs`
- Modify: `src/main.rs` (wire `depth` and `impact` commands)

- [ ] **Step 1: Add depth to analytics/mod.rs**

```rust
pub mod amounts;
pub mod pnl;
pub mod greeks;
pub mod depth;
```

- [ ] **Step 2: Write failing tests and implementation in src/analytics/depth.rs**

```rust
/// One price level with its total active liquidity.
#[derive(Debug, Clone)]
pub struct LiquidityLevel {
    pub price: f64,
    pub liquidity: u128,
}

/// Estimated price impact for a trade.
#[derive(Debug, Clone)]
pub struct PriceImpact {
    pub target_pct: f64,
    pub target_price: f64,
    pub usd_needed: f64,
}

/// Build a bucketed liquidity distribution around the current price.
///
/// tick_liquidities: (tick_index, net_liquidity_delta) pairs from tick array accounts.
/// Uses the net-liquidity-delta model: liquidity at a tick = sum of all deltas at or below it.
/// For now, accepts an empty slice — pool-level liquidity is used as a fallback in callers.
pub fn build_distribution(
    tick_liquidities: &[(i32, i64)],
    current_tick: i32,
    tick_spacing: i32,
    n_buckets_each_side: usize,
) -> Vec<LiquidityLevel> {
    if tick_liquidities.is_empty() {
        return vec![];
    }

    let mut result = Vec::with_capacity(n_buckets_each_side * 2 + 1);
    let mut running: i128 = 0;

    for i in 0..=(n_buckets_each_side * 2) {
        let bucket_start = current_tick
            - (n_buckets_each_side as i32) * tick_spacing
            + i as i32 * tick_spacing;
        let bucket_end = bucket_start + tick_spacing;

        let delta: i64 = tick_liquidities
            .iter()
            .filter(|(t, _)| *t >= bucket_start && *t < bucket_end)
            .map(|(_, d)| *d)
            .sum();

        running += delta as i128;

        let mid_tick = bucket_start + tick_spacing / 2;
        let price = 1.0001f64.powi(mid_tick);

        result.push(LiquidityLevel {
            price,
            liquidity: running.unsigned_abs(),
        });
    }

    result
}

/// Estimate USD trade size needed to move price by target_pct%.
///
/// Uses the CLMM constant-liquidity approximation:
///   buy token A:  amount_a = L * (1/sqrt(P) - 1/sqrt(P_target))
///   USD cost ≈ amount_a * P_current
pub fn estimate_impact(
    current_price: f64,
    liquidity: u128,
    target_pct: f64,
    is_buy: bool,
) -> PriceImpact {
    let l = liquidity as f64;
    let target_price = if is_buy {
        current_price * (1.0 + target_pct / 100.0)
    } else {
        current_price * (1.0 - target_pct / 100.0)
    };

    let sqrt_p = current_price.sqrt();
    let sqrt_target = target_price.sqrt();
    let amount_a = l * (1.0 / sqrt_p - 1.0 / sqrt_target).abs();
    let usd_needed = amount_a * current_price;

    PriceImpact { target_pct, target_price, usd_needed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_ticks_returns_empty() {
        let result = build_distribution(&[], 0, 64, 4);
        assert!(result.is_empty());
    }

    #[test]
    fn test_higher_liquidity_needs_larger_trade_for_same_impact() {
        let small = estimate_impact(100.0, 1_000, 1.0, true);
        let large = estimate_impact(100.0, 1_000_000_000, 1.0, true);
        assert!(large.usd_needed > small.usd_needed);
    }

    #[test]
    fn test_larger_pct_needs_more_usd() {
        let one_pct = estimate_impact(100.0, 1_000_000, 1.0, true);
        let five_pct = estimate_impact(100.0, 1_000_000, 5.0, true);
        assert!(five_pct.usd_needed > one_pct.usd_needed);
    }

    #[test]
    fn test_target_price_correct_direction_for_buy() {
        let impact = estimate_impact(100.0, 1_000_000, 2.0, true);
        assert!(impact.target_price > 100.0);
    }

    #[test]
    fn test_target_price_correct_direction_for_sell() {
        let impact = estimate_impact(100.0, 1_000_000, 2.0, false);
        assert!(impact.target_price < 100.0);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test analytics::depth`

Expected: all 5 tests pass.

- [ ] **Step 4: Wire depth command in main.rs**

Replace `Commands::Depth` arm:

```rust
Commands::Depth { pool } => {
    let rpc = rpc::SolanaRpc::new(&cli.rpc_url);
    let pool_data = rpc.fetch_account_data(pool)?;
    let pool_state = protocols::orca::parse_pool(&pool_data)?;

    let to_price = |sqrt_q64: u128| -> f64 {
        let s = sqrt_q64 as f64 / (1u128 << 64) as f64;
        s * s
    };
    let price_current = to_price(pool_state.sqrt_price);

    println!(
        "Liquidity Distribution  (pool liquidity: {:.0}M)",
        pool_state.liquidity as f64 / 1e6
    );
    println!("{}", "─".repeat(50));

    // Tick arrays not fetched yet — show price impact using pool-level liquidity
    for pct in [1.0f64, 2.0, 5.0] {
        let buy = analytics::depth::estimate_impact(price_current, pool_state.liquidity, pct, true);
        let sell = analytics::depth::estimate_impact(price_current, pool_state.liquidity, pct, false);
        println!(
            "  +{:.0}%  (~${:.4}): ${:.0} needed to buy  |  ${:.0} needed to sell",
            pct, buy.target_price, buy.usd_needed, sell.usd_needed
        );
    }

    println!();
    println!("Note: uses pool-level liquidity. Tick-array depth map coming in a future update.");
}
```

Replace `Commands::Impact` arm:

```rust
Commands::Impact { pool, size } => {
    let rpc = rpc::SolanaRpc::new(&cli.rpc_url);
    let pool_data = rpc.fetch_account_data(pool)?;
    let pool_state = protocols::orca::parse_pool(&pool_data)?;

    let to_price = |sqrt_q64: u128| -> f64 {
        let s = sqrt_q64 as f64 / (1u128 << 64) as f64;
        s * s
    };
    let price_current = to_price(pool_state.sqrt_price);

    // Solve for price impact given USD trade size
    // amount_a = size / price; price_impact = (P_entry/sqrt(P)^2 - size/L)
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
```

- [ ] **Step 5: Build and test**

```bash
cargo build
cargo run -- depth <DEVNET_POOL_ADDRESS>
cargo run -- impact <DEVNET_POOL_ADDRESS> --size 10000
```

Expected: price impact estimates printed.

- [ ] **Step 6: Commit**

```bash
git add src/analytics/depth.rs src/analytics/mod.rs src/main.rs
git commit -m "feat: add depth and impact commands with price impact estimation"
```

---

## Task 10: Raydium CLMM support

**Files:**
- Modify: `src/protocols/mod.rs`
- Create: `src/protocols/raydium.rs`

- [ ] **Step 1: Add raydium to protocols/mod.rs**

```rust
pub mod orca;
pub mod raydium;
```

- [ ] **Step 2: Write failing tests and implementation in src/protocols/raydium.rs**

```rust
use anyhow::{anyhow, Result};
use borsh::BorshDeserialize;
use solana_sdk::pubkey::Pubkey;

pub const RAYDIUM_CLMM_PROGRAM_ID: &str = "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK";

const DISC: usize = 8;

/// Key fields from a Raydium CLMM PoolState account.
///
/// IMPORTANT: Verify field order against the actual program source before
/// testing on mainnet. Borsh is order-sensitive.
/// Source: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/states/pool.rs
#[derive(BorshDeserialize, Debug, Clone)]
pub struct RaydiumPool {
    pub bump: [u8; 1],
    pub amm_config: Pubkey,
    pub owner: Pubkey,
    pub token_mint_0: Pubkey,
    pub token_mint_1: Pubkey,
    pub token_vault_0: Pubkey,
    pub token_vault_1: Pubkey,
    pub observation_key: Pubkey,
    pub mint_decimals_0: u8,
    pub mint_decimals_1: u8,
    pub tick_spacing: u16,
    pub liquidity: u128,
    pub sqrt_price_x64: u128,   // same Q64.64 format as Orca
    pub tick_current: i32,
    // remaining fields omitted
}

/// Key fields from a Raydium CLMM PersonalPositionState account.
#[derive(BorshDeserialize, Debug, Clone)]
pub struct RaydiumPosition {
    pub bump: [u8; 1],
    pub nft_mint: Pubkey,
    pub pool_id: Pubkey,
    pub tick_lower_index: i32,
    pub tick_upper_index: i32,
    pub liquidity: u128,
    pub fee_growth_inside_0_last_x64: u128,
    pub fee_growth_inside_1_last_x64: u128,
    pub token_fees_owed_0: u64,
    pub token_fees_owed_1: u64,
}

pub fn parse_pool(data: &[u8]) -> Result<RaydiumPool> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    RaydiumPool::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Raydium pool: {}", e))
}

pub fn parse_position(data: &[u8]) -> Result<RaydiumPosition> {
    if data.len() < DISC {
        return Err(anyhow!("Account data too short: {} bytes", data.len()));
    }
    RaydiumPosition::try_from_slice(&data[DISC..])
        .map_err(|e| anyhow!("Failed to deserialize Raydium position: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pool_too_short_returns_error() {
        let result = parse_pool(&[0u8; 4]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn test_parse_position_too_short_returns_error() {
        let result = parse_position(&[0u8; 4]);
        assert!(result.is_err());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test protocols::raydium`

Expected: both tests pass.

- [ ] **Step 4: Add raydium arm to position command in main.rs**

In the `Commands::Position` match, add after the `"orca"` arm:

```rust
"raydium" => {
    let rpc = rpc::SolanaRpc::new(&cli.rpc_url);

    // Raydium position PDA: seeds = ["position", nft_mint]
    let raydium_program = Pubkey::from_str(protocols::raydium::RAYDIUM_CLMM_PROGRAM_ID)?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let (position_pda, _) = Pubkey::find_program_address(
        &[b"position", mint_pubkey.as_ref()],
        &raydium_program,
    );

    let position_data = rpc.fetch_account_data(&position_pda.to_string())?;
    let pos = protocols::raydium::parse_position(&position_data)?;

    let pool_data = rpc.fetch_account_data(&pos.pool_id.to_string())?;
    let pool = protocols::raydium::parse_pool(&pool_data)?;

    // Reuse same analytics — sqrt_price format is identical (Q64.64)
    let to_price = |sqrt_q64: u128| -> f64 {
        let s = sqrt_q64 as f64 / (1u128 << 64) as f64;
        s * s
    };
    let price_current = to_price(pool.sqrt_price_x64);

    println!("Raydium Position: {}", position_pda);
    println!("Pool:     {}", pos.pool_id);
    println!("Price:    ${:.4}", price_current);
    println!("Tick:     {} (range: {} — {})", pool.tick_current, pos.tick_lower_index, pos.tick_upper_index);
    println!("Liquidity: {}", pos.liquidity);
    // Full table output (same as Orca) — left as exercise to unify with display::table
}
```

- [ ] **Step 5: Build**

Run: `cargo build`

Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add src/protocols/raydium.rs src/protocols/mod.rs src/main.rs
git commit -m "feat: add Raydium CLMM account deserialization and basic position command"
```

---

## Task 11: `watch` command (real-time WebSocket)

**Files:**
- Modify: `src/main.rs` (replace watch TODO)

- [ ] **Step 1: Replace Commands::Watch arm in main()**

```rust
Commands::Watch { mint } => {
    use tokio_tungstenite::connect_async;
    use futures_util::StreamExt;

    let rpc = rpc::SolanaRpc::new(&cli.rpc_url);
    let whirlpool_program = Pubkey::from_str(protocols::orca::WHIRLPOOL_PROGRAM_ID)?;
    let mint_pubkey = Pubkey::from_str(mint)?;
    let (position_pda, _) = Pubkey::find_program_address(
        &[b"position", mint_pubkey.as_ref()],
        &whirlpool_program,
    );

    let position_data = rpc.fetch_account_data(&position_pda.to_string())?;
    let pos = protocols::orca::parse_position(&position_data)?;
    let pool_addr = pos.whirlpool.to_string();

    let ws_url = cli.rpc_url
        .replace("https://", "wss://")
        .replace("http://", "ws://");

    println!("Watching {}  (Ctrl+C to stop)", mint);
    println!("WebSocket: {}", ws_url);

    let (mut ws, _) = connect_async(&ws_url).await
        .map_err(|e| anyhow::anyhow!("WebSocket connect failed: {}", e))?;

    let subscribe = serde_json::json!({
        "jsonrpc": "2.0", "id": 1,
        "method": "accountSubscribe",
        "params": [pool_addr, {"encoding": "base64", "commitment": "confirmed"}]
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(subscribe.to_string())).await?;

    println!("Subscribed. Waiting for updates...\n");

    while let Some(msg) = ws.next().await {
        let text = match msg? {
            tokio_tungstenite::tungstenite::Message::Text(t) => t,
            _ => continue,
        };

        let json: serde_json::Value = serde_json::from_str(&text)?;
        if json["method"] != "accountNotification" {
            continue;
        }

        // Clear terminal and reprint
        print!("\x1B[2J\x1B[1;1H");
        println!("[{}] Pool update received", chrono::Utc::now().format("%H:%M:%S UTC"));
        println!();

        let pool_data = rpc.fetch_account_data(&pool_addr)?;
        let pool = protocols::orca::parse_pool(&pool_data)?;

        let to_price = |sqrt_q64: u128| -> f64 {
            let s = sqrt_q64 as f64 / (1u128 << 64) as f64;
            s * s
        };

        let price_current = to_price(pool.sqrt_price);
        let in_range = pool.tick_current_index >= pos.tick_lower_index
            && pool.tick_current_index <= pos.tick_upper_index;

        println!("Pool:     {}", pool_addr);
        println!("Price:    ${:.4}", price_current);
        println!("Tick:     {}", pool.tick_current_index);
        println!("In range: {}", if in_range { "✓ YES" } else { "✗ NO — needs rebalance" });
        println!("Liquidity: {}", pool.liquidity);
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo build`

Expected: compiles.

- [ ] **Step 3: Test on devnet**

```bash
cargo run -- watch <DEVNET_POSITION_MINT>
```

Expected: connects to WebSocket, prints "Subscribed", clears and reprints on each pool update. On quiet devnet may take several minutes for an update — swap on the pool to trigger one.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: watch command with real-time WebSocket pool updates"
```

---

## Task 12: Full test suite + clippy + README

**Files:**
- Create: `README.md`

- [ ] **Step 1: Run full test suite**

Run: `cargo test`

Expected: all tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`

Expected: no warnings. Fix any that appear.

- [ ] **Step 3: Write README.md**

```markdown
# lp-inspect

CLI tool for inspecting CLMM positions on Solana. Like `cast` from Foundry, but for LP positions.

Supports: Orca Whirlpools, Raydium CLMM.

## Install

```bash
cargo install --git https://github.com/YOUR_USERNAME/tick-liq
```

## Usage

```bash
# Full P&L breakdown of an Orca position
lp-inspect position <POSITION_MINT>

# Raydium position
lp-inspect position <POSITION_MINT> --protocol raydium

# Watch position in real-time (WebSocket)
lp-inspect watch <POSITION_MINT>

# Liquidity depth + price impact
lp-inspect depth <POOL_ADDRESS>

# Price impact for a specific trade size (USD)
lp-inspect impact <POOL_ADDRESS> --size 50000
```

## Example output

```
Position: whiRLbMi...  (Orca 30.00 bps)
─────────────────────────────────────────
Range:      $142.5000 — $158.3000
Current:    $151.2000  ✓ IN RANGE  (63%)

Amounts:    12.345678 A  +  456.78 B

P&L:
  Fees:  +23.45  (+1.25%)
  IL:     -8.90  (-0.48%)
  Net:   +14.55  (+0.77%)

Delta: -0.3400   Gamma: 0.020000
```

## Environment

```bash
export SOLANA_RPC_URL=https://your-rpc-url.com   # default: devnet
```

## License

MIT
```

- [ ] **Step 4: Commit and push**

```bash
git add README.md
git commit -m "docs: add README with install and usage"
git push origin master
```
