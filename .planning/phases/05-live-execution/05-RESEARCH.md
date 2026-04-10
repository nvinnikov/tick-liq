# Phase 5: Live Execution - Research

**Researched:** 2026-04-10
**Domain:** Orca Whirlpool CPI / Solana transaction building from Rust client binary
**Confidence:** MEDIUM (core account structures HIGH; Cargo version conflict details MEDIUM due to git-crate nature)

---

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
- **Orca CPI approach:** Add `whirlpool-cpi` (git, `anchor/0.29.0` branch, `cpi` feature) + `anchor-client = "0.29"` + `anchor-lang = "0.29"` — use IDL-generated instruction builders: `whirlpool_cpi::cpi::close_position`, `collect_fees`, `open_position`
- **Drift hedge in Phase 5:** Stub only — `tracing::info!` log of computed size, no Drift deps, no Drift CPI
- **LIVE-02 and LIVE-04:** Deferred to a later phase — out of scope
- **Atomicity model:** Three separate Solana txns (close → collect → open); on `open_position` failure: retry once (2s delay), then halt + CRITICAL log; `close_position` and `collect_fees` do not retry
- **Error persistence:** Reuse `shadow_rebalances` table; `trigger_reason = 'live_rebalance_failed'`, `error_flag = true`, `error_message` = anyhow error chain string
- **Keypair loading:** `WALLET_KEYPAIR` env var, JSON byte array `[u8; 64]` format; `Keypair::from_bytes()`; process exits at startup if absent
- **Test strategy:** `simulateTransaction` RPC — no funded devnet wallet; tests are `#[ignore]` by default, enabled with `-- --include-ignored` when `WALLET_KEYPAIR` and `RPC_URL` are set

### Claude's Discretion
- Exact account derivation helpers (PDA seeds for position NFT mint, tick array PDAs, token vaults)
- `CpiContext` account struct layout (which accounts to pass for each instruction)
- Whether to extract an `OrcaExecutor` struct or keep CPI calls inline in `main.rs`
- Tracing span structure for the live rebalance cycle

### Deferred Ideas (OUT OF SCOPE)
- Drift perp hedge real CPI (LIVE-02) — no Drift deps in Phase 5
- LIVE-04 atomicity between LP and Drift hedge
- Devnet integration test with funded wallet
- Jito bundle for MEV-protected rebalance
</user_constraints>

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| LIVE-01 | Rebalance executes close → collect fees → open sequence via Anchor CPI to Orca Whirlpool program | Orca CPI account structs, instruction builders, 4-step sequence (update_fees_and_rewards must precede collect_fees), PDA derivations |
| LIVE-03 | Keypair loaded exclusively from env var (`WALLET_KEYPAIR`); process exits if var absent | `Keypair::from_bytes()` API, startup guard pattern, clap `env` attribute |
</phase_requirements>

---

## Summary

Phase 5 wires real Orca Whirlpool close → collect → open transactions from a Rust client binary (not from inside an Anchor on-chain program). This distinction matters: the `whirlpool-cpi` crate targets on-chain CPI use, but its anchor 0.29 branch also exports `Instruction` objects that can be submitted client-side via `solana_client::rpc_client::RpcClient`. The planner must treat tick-liq as an off-chain binary that constructs `solana_sdk::transaction::Transaction` objects and submits them via the existing `SolanaRpc` client.

The key architectural decision is that `whirlpool-cpi` (git, `anchor/0.29.0`) is an Anchor-compatible CPI adapter crate that exposes instruction discriminators and account layouts matching the Orca Whirlpool program. Because tick-liq is not itself an Anchor program, the actual submission path is `solana_client::rpc_client::RpcClient::send_and_confirm_transaction()` — the same client already wired in `src/rpc/`. The `anchor-client` crate is for convenience but the raw `Instruction` + `Transaction` + `send_and_confirm_transaction` path works without it for a binary.

The most critical pitfall is the **mandatory 4-step sequence**: `update_fees_and_rewards` must be called before `collect_fees`, or the position's `fee_owed_a` / `fee_owed_b` will still be zero and collection will succeed but return nothing. The locked 3-step sequence in CONTEXT.md (close → collect → open) is correct but must be expanded to 4 steps: close → update_fees_and_rewards → collect_fees → open_position.

**Primary recommendation:** Implement an `OrcaExecutor` struct in `src/execution/` that holds the keypair and RPC URL, with four async methods (`close_position`, `update_fees_and_rewards`, `collect_fees`, `open_position`) each returning `anyhow::Result<Signature>`. Wire this into the `ShadowGuard::Live` branch in `main.rs`.

---

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `whirlpool-cpi` | 0.1.4 (git, `anchor/0.29.0` branch) | IDL-generated Orca Whirlpool instruction builders and account types | Official Orca CPI adapter; matches Whirlpool program's Anchor 0.29 ABI; locked in CONTEXT.md |
| `anchor-lang` | `=0.29.0` | Account macros and discriminators used by whirlpool-cpi | Pin-matched to whirlpool-cpi; Whirlpool program was compiled with 0.29 |
| `anchor-spl` | `=0.29.0` | SPL token account helpers used by whirlpool-cpi | Pair-dep with anchor-lang 0.29 |
| `solana-client` | `^1.18` | Transaction submission, blockhash fetching, `simulate_transaction` | Already in Cargo.toml; `RpcClient::send_and_confirm_transaction` is the submission path |
| `solana-sdk` | `^1.18` | `Transaction`, `Keypair`, `Pubkey`, `Instruction`, `AccountMeta` | Already in Cargo.toml |

### Supporting

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `spl-token` | `~3.5` (pulled by anchor-spl 0.29) | SPL Token program ID constant for account metas | Needed to pass `token_program` account in instruction builders |
| `spl-associated-token-account` | pulled by anchor-spl | ATA derivation for owner token accounts | Needed for token_owner_account_a/b derivation |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `whirlpool-cpi` git | `orca_whirlpools_client 7.2` | orca_whirlpools_client 7.2 requires `solana-program ^3` which conflicts with `solana-sdk 1.18`; **not viable** without a major Solana SDK upgrade |
| `whirlpool-cpi` git | Hand-rolled Anchor discriminator + borsh | Fragile; any IDL change breaks silently; whirlpool-cpi provides tested discriminators |
| `anchor-client` `RequestBuilder` | Direct `solana_client` + `Transaction` | `anchor-client` adds request building convenience but not needed for a fixed set of instructions; direct client is simpler |

**Installation additions to Cargo.toml:**
```toml
# Orca Whirlpool CPI (Anchor 0.29 branch — pinned to avoid anchor-lang drift)
whirlpool-cpi = { git = "https://github.com/orca-so/whirlpool-cpi", branch = "anchor/0.29.0", features = ["cpi"] }
anchor-lang  = "=0.29.0"
anchor-spl   = "=0.29.0"

# [patch.crates-io] may be required — see Cargo version conflict section below
```

**Version verification:**
- `whirlpool-cpi` v0.1.4 on `anchor/0.29.0` branch [VERIFIED: crates.io raw Cargo.toml]
- `anchor-lang` and `anchor-client` latest stable is 1.0.0 (2026-04-02) [VERIFIED: crates.io API]; use `=0.29.0` pin per locked decision
- `solana-client` / `solana-sdk` 1.18 already present [VERIFIED: project Cargo.toml]

---

## Architecture Patterns

### Recommended Project Structure Addition

```
src/
├── execution/
│   ├── mod.rs              # add pub use orca_executor::OrcaExecutor
│   ├── rebalance.rs        # existing dry-run plan (keep)
│   ├── hedge.rs            # add tracing::info! log of computed size
│   ├── shadow_guard.rs     # existing guard (keep; Live branch gains real submit)
│   └── orca_executor.rs    # NEW: OrcaExecutor struct with 4 methods
├── protocols/
│   └── orca.rs             # add position_pda() and token_vault helpers
```

### Pattern 1: Client-side Instruction Building (NOT on-chain CPI)

**What:** tick-liq is a binary, not an on-chain program. "CPI" in `whirlpool-cpi` means the crate generates instruction discriminators compatible with the Whirlpool program. Submission uses `RpcClient::send_and_confirm_transaction`, not `solana_program::program::invoke`.

**When to use:** Always — this is the only valid pattern for an off-chain Rust binary calling an Anchor program.

**Pattern:**
```rust
// Source: solana-client 1.18 docs + whirlpool-cpi anchor/0.29 branch structure
// Build an Instruction for close_position
let close_ix = whirlpool_cpi::instruction::ClosePosition {
    // discriminator is embedded in the instruction data by the cpi crate
};
let accounts = whirlpool_cpi::accounts::ClosePosition {
    position_authority: wallet.pubkey(),
    receiver: wallet.pubkey(),
    position: position_pda,
    position_mint: position_mint_pubkey,
    position_token_account: position_ata,
    token_program: spl_token::ID,
};
let ix = solana_sdk::instruction::Instruction {
    program_id: whirlpool_program_pubkey(),
    accounts: accounts.to_account_metas(None),  // anchor_lang::ToAccountMetas
    data: close_ix.data(),                        // anchor_lang::InstructionData
};
let blockhash = rpc_client.get_latest_blockhash()?;
let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(
    &[ix],
    Some(&wallet.pubkey()),
    &[&wallet],
    blockhash,
);
rpc_client.send_and_confirm_transaction(&tx)?;
```

**Note:** This pattern uses `anchor_lang::ToAccountMetas` and `anchor_lang::InstructionData` traits from the whirlpool-cpi crate's re-exported anchor-lang 0.29. `[ASSUMED]` — the exact trait method names need verification against the actual whirlpool-cpi source after adding the dependency.

### Pattern 2: The 4-Step Sequence (Critical Correction to CONTEXT.md)

**What:** CONTEXT.md describes 3 steps (close → collect → open). In practice, collecting fees requires 4 steps. The Orca program does not auto-update fee checkpoints in `collect_fees` — a separate `update_fees_and_rewards` instruction must precede it.

**Steps:**
1. `close_position` — burns position NFT, reclaims rent; position account is closed
2. `update_fees_and_rewards` — writes `fee_owed_a` / `fee_owed_b` into position account; requires `tick_array_lower` + `tick_array_upper`
3. `collect_fees` — transfers `fee_owed_a` / `fee_owed_b` from pool vaults to owner token accounts
4. `open_position` — creates new position NFT + position account; position mint is a fresh keypair

**Important:** After `close_position`, the position account is closed. This means `update_fees_and_rewards` and `collect_fees` must happen BEFORE `close_position`, or there is no position to update/collect from. The correct order is:

```
update_fees_and_rewards → collect_fees → close_position → open_position
```

[VERIFIED: Orca Whirlpool program source — update_fees_and_rewards.rs requires position account to be open; close_position.rs verifies position is empty (liquidity=0, fees=0)]

**This changes the CONTEXT.md sequence.** The locked decision says "close → collect → open" but `close_position` requires the position to be empty (no pending fees). The correct operator sequence for a rebalance with fee collection is:

```
update_fees_and_rewards → collect_fees → decrease_liquidity → close_position → open_position
```

However, since the existing `WhirlpoolPosition` struct shows `fee_owed_a` and `fee_owed_b` (pre-fetched values), and since the CONTEXT.md sequence was already discussed by the user, the planner should flag this ordering issue. For Phase 5 it is safe to adopt:

```
update_fees_and_rewards → collect_fees → close_position → open_position
```

assuming liquidity has already been decreased (or the position is already near-empty by the time a rebalance fires). [ASSUMED] that in Phase 5 the existing LP position has already had liquidity decreased before close_position is called, matching the Orca program constraint that close_position requires zero liquidity.

### Pattern 3: Position PDA Derivation

**What:** The Whirlpool position account address is a PDA derived from seeds `[b"position", position_mint_pubkey.as_ref()]` under the Whirlpool program.

```rust
// Source: [VERIFIED: Orca Whirlpool program source, open_position.rs]
let (position_pda, _bump) = Pubkey::find_program_address(
    &[b"position", position_mint.as_ref()],
    &whirlpool_program_pubkey(),
);
```

This derivation is already implemented in `src/protocols/orca.rs` for tick_array PDAs; the same pattern applies.

### Pattern 4: Keypair Loading at Startup

**What:** Load `WALLET_KEYPAIR` env var (JSON `[u8; 64]` byte array) and exit process if absent.

```rust
// Source: [VERIFIED: solana_sdk::signer::keypair::Keypair::from_bytes docs]
fn load_wallet_keypair() -> anyhow::Result<Keypair> {
    let raw = std::env::var("WALLET_KEYPAIR")
        .map_err(|_| anyhow::anyhow!("WALLET_KEYPAIR env var not set; process cannot start in live mode"))?;
    let bytes: Vec<u8> = serde_json::from_str(&raw)
        .context("WALLET_KEYPAIR must be a JSON array of 64 bytes")?;
    Keypair::from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Invalid keypair bytes: {}", e))
}
```

Call this at the top of the `Watch` arm when `run_mode == RunMode::Live`, before the shadow gate, and call `std::process::exit(1)` on error.

### Pattern 5: simulateTransaction for Tests

**What:** `RpcClient::simulate_transaction` validates instruction serialization and account derivation without consuming SOL.

```rust
// Source: [VERIFIED: solana_client 1.18 docs — simulate_transaction method]
let result = rpc_client.simulate_transaction(&tx)?;
// Success check: result.value.err must be None
assert!(
    result.value.err.is_none(),
    "simulateTransaction failed: {:?}\nlogs: {:?}",
    result.value.err,
    result.value.logs,
);
```

Return type: `Result<Response<RpcSimulateTransactionResult>, ClientError>`

Key fields of `RpcSimulateTransactionResult`:
- `err: Option<TransactionError>` — `None` on success
- `logs: Option<Vec<String>>` — program log lines (useful for diagnosing failures)
- `units_consumed: Option<u64>` — CU budget consumed

### Anti-Patterns to Avoid

- **Calling `collect_fees` before `update_fees_and_rewards`:** Returns 0 tokens silently. Always update first.
- **Calling `close_position` on a position with non-zero liquidity:** Program will reject with `LiquidityNonZero` error. Must have zero liquidity before close.
- **Using `orca_whirlpools_client 7.x` with solana-sdk 1.18:** Requires solana-program ^3 — hard version conflict. Use `whirlpool-cpi` git branch instead.
- **Generating a new position_mint from a deterministic seed:** Position mint must be a fresh `Keypair` (random) — it signs the `open_position` tx. Do not use `find_program_address` for the mint.
- **Ignoring the `anchor-lang` version pin:** Using `anchor-lang` any version other than `=0.29.0` with `whirlpool-cpi anchor/0.29.0` will cause type mismatches at the `ToAccountMetas` / `InstructionData` boundaries.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Whirlpool instruction discriminators | Custom 8-byte discriminator encoding | `whirlpool-cpi` crate | Discriminators are sha256("global:instruction_name")[0..8]; any typo = silent program error |
| ATA derivation | Manual PDA with token program seeds | `spl_associated_token_account::get_associated_token_address()` | Seeds are complex; library is already pulled by anchor-spl |
| Blockhash caching | Custom blockhash freshness logic | `RpcClient::get_latest_blockhash()` per transaction | Blockhash validity window is ~150 slots (~60s); caching risks `BlockhashNotFound` |
| Position account layout | Custom borsh schema | `WhirlpoolPosition` in `src/protocols/orca.rs` (already exists) | Correct layout already implemented and tested |
| Tick array PDA derivation | Manual seeds | `tick_array_pda()` in `src/protocols/orca.rs` (already exists) | Already implemented correctly |

**Key insight:** The account layout work is already done in `src/protocols/orca.rs`. Phase 5 adds instruction building and transaction submission on top of the existing parsing layer.

---

## Cargo Version Conflict: Critical Information

### The Conflict

The `whirlpool-cpi` crate at `anchor/0.29.0` depends on `anchor-lang = "=0.29.0"` which depends on `solana-program ~1.17`. The tick-liq project uses `solana-sdk 1.18` which provides `solana-program 1.18`. These should be compatible (both 1.x), but the `orca_whirlpools_client 7.2` crate (the alternative) requires `solana-program ^3` which is definitively incompatible.

[VERIFIED: orca_whirlpools_client Cargo.toml via GitHub raw] — requires `solana-pubkey ^3`, `solana-instruction ^3`, etc.

### The whirlpool-cpi + anchor 0.29 + solana 1.18 Compatibility

The `whirlpool-cpi` crate on `anchor/0.29.0` was tested with Solana 1.17.14-1.18.17 [CITED: whirlpools-cpi-examples README]. Solana SDK 1.18 is within the tested range.

**Potential issue:** Orca's `whirlpools-cpi-examples` README documents that for Anchor 0.29 you may need:
```bash
cargo update solana-program@<LATEST_V2_OR_V3> --precise 1.18.17
```
This only applies if a transitive dependency pulls in solana-program v2 or v3. Since tick-liq only has `solana-sdk 1.18` and `orca_whirlpools_core 2` (math-only, no solana-program dep), this conflict is likely absent. Verify with `cargo tree -d solana-program` after adding whirlpool-cpi.

[ASSUMED] that the version conflict will NOT affect tick-liq because `orca_whirlpools_core` is a pure math crate without solana-program dependency. Confirm with `cargo tree -d` after adding the dep.

---

## Accounts Reference (Complete)

### `close_position`

| Account | Writable | Signer | Source |
|---------|----------|--------|--------|
| `position_authority` | No | YES | wallet keypair |
| `receiver` | YES | No | wallet pubkey (or any SOL destination) |
| `position` | YES | No | PDA: `["position", position_mint]` under Whirlpool program |
| `position_mint` | YES | No | position NFT mint pubkey (from WhirlpoolPosition._position_mint) |
| `position_token_account` | YES | No | ATA of position_authority for position_mint |
| `token_program` | No | No | `spl_token::ID` |

[VERIFIED: Orca Whirlpool program source, close_position.rs via GitHub]

### `update_fees_and_rewards` (must precede `collect_fees`)

| Account | Writable | Signer | Source |
|---------|----------|--------|--------|
| `whirlpool` | YES | No | pool address |
| `position` | YES | No | PDA as above |
| `tick_array_lower` | No | No | PDA: `["tick_array", whirlpool, start_tick_lower.to_string()]` |
| `tick_array_upper` | No | No | PDA: `["tick_array", whirlpool, start_tick_upper.to_string()]` |

`tick_array_lower` = `tick_array_pda(&whirlpool, tick_array_start_index(pos.tick_lower_index, tick_spacing))`
`tick_array_upper` = `tick_array_pda(&whirlpool, tick_array_start_index(pos.tick_upper_index, tick_spacing))`

Both helpers already exist in `src/protocols/orca.rs`. [VERIFIED: existing code + Whirlpool program source]

### `collect_fees`

| Account | Writable | Signer | Source |
|---------|----------|--------|--------|
| `whirlpool` | No | No | pool address |
| `position_authority` | No | YES | wallet keypair |
| `position` | YES | No | PDA as above |
| `position_token_account` | No | No | ATA of position_authority for position_mint |
| `token_owner_account_a` | YES | No | ATA of wallet for token_mint_a |
| `token_vault_a` | YES | No | from WhirlpoolPool._token_vault_a |
| `token_owner_account_b` | YES | No | ATA of wallet for token_mint_b |
| `token_vault_b` | YES | No | from WhirlpoolPool._token_vault_b |
| `token_program` | No | No | `spl_token::ID` |

[VERIFIED: Orca Whirlpool program source, collect_fees.rs + CollectFeesCpiAccounts via docs.rs]

**Note:** `token_vault_a` and `token_vault_b` are fields on the `WhirlpoolPool` struct. However, looking at `src/protocols/orca.rs`, these are stored as `_token_vault_a` and `_token_vault_b` (private-prefix convention for borsh positional fields). The Rust code will need to fetch these from the pool account. The existing `WhirlpoolPool` struct already deserializes them — they just use the `_` prefix naming. To use in instruction building, access as `pool._token_vault_a` directly, or add public accessor methods.

Similarly `_token_mint_a` and `_token_mint_b` are needed to derive the owner token ATAs. These are also present in the existing struct.

### `open_position`

| Account | Writable | Signer | Source |
|---------|----------|--------|--------|
| `funder` | YES | YES | wallet keypair |
| `owner` | No | No | wallet pubkey |
| `position` | YES | No | PDA: `["position", new_position_mint]` |
| `position_mint` | YES | YES | fresh `Keypair::new()` — must sign the tx |
| `position_token_account` | YES | No | ATA of owner for new position_mint |
| `whirlpool` | No | No | pool address |
| `token_program` | No | No | `spl_token::ID` |
| `system_program` | No | No | `system_program::ID` |
| `rent` | No | No | `sysvar::rent::ID` |
| `associated_token_program` | No | No | `spl_associated_token_account::ID` |

**Arguments:** `tick_lower_index: i32`, `tick_upper_index: i32`

[VERIFIED: Orca Whirlpool program source, open_position.rs + OpenPositionCpiAccounts via docs.rs]

**Critical:** The `position_mint` must be a brand-new `Keypair` each time. It signs the transaction. Derive the position PDA deterministically from this new mint pubkey.

---

## Common Pitfalls

### Pitfall 1: collect_fees Returns Zero Without update_fees_and_rewards

**What goes wrong:** `collect_fees` succeeds (no error), but transfers 0 tokens.
**Why it happens:** The Whirlpool program reads `fee_owed_a` and `fee_owed_b` directly from the position account. These fields are only written by `update_fees_and_rewards`. Without it, they remain at their last checkpoint value (often 0 for new positions or after a previous collect).
**How to avoid:** Always call `update_fees_and_rewards` immediately before `collect_fees` in the same rebalance cycle.
**Warning signs:** Position shows accrued fees in monitoring, but collect transfers nothing.

### Pitfall 2: close_position Requires Zero Liquidity

**What goes wrong:** `close_position` fails with program error `LiquidityNonZero` or similar.
**Why it happens:** The Orca program has an invariant that a position must be empty (zero liquidity, zero owed fees) before it can be closed.
**How to avoid:** The Phase 5 flow assumes the position has already had its liquidity removed (decrease_liquidity must precede close_position in a full rebalance). Phase 5 should verify `pos.liquidity == 0` before calling close_position, and emit a CRITICAL log + halt if non-zero.
**Warning signs:** Position still has `liquidity > 0` in the `WhirlpoolPosition` struct at execution time.

### Pitfall 3: anchor-lang Version Drift

**What goes wrong:** Compilation errors at `ToAccountMetas` or `InstructionData` trait boundaries; type mismatches between anchor-lang versions.
**Why it happens:** `whirlpool-cpi` at `anchor/0.29.0` was compiled against `anchor-lang = "=0.29.0"`. If the project adds `anchor-lang = "0.30"` (or any non-0.29 version), Cargo may resolve two different anchor-lang versions, causing type incompatibilities.
**How to avoid:** Use `anchor-lang = "=0.29.0"` (exact pin) in Cargo.toml. Run `cargo tree | grep anchor-lang` after adding deps to confirm only one version is resolved.
**Warning signs:** Compilation errors mentioning `anchor_lang` traits not being satisfied.

### Pitfall 4: position_mint Must Sign open_position

**What goes wrong:** Transaction rejected with `MissingRequiredSignature`.
**Why it happens:** The Orca program requires the new position_mint keypair to sign the `open_position` instruction (to prove the caller controls the mint). This is different from most accounts.
**How to avoid:** Pass both `wallet` and `new_position_mint_keypair` as signers: `Transaction::new_signed_with_payer(&[open_ix], Some(&wallet.pubkey()), &[&wallet, &new_mint], blockhash)`.
**Warning signs:** Simulation shows `MissingRequiredSignature` for the mint account.

### Pitfall 5: Tick Array Start Index Must Use Euclidean Division

**What goes wrong:** Negative tick indexes produce wrong tick array start with integer division.
**Why it happens:** Rust's `%` operator is remainder, not modulus. Negative ticks like -100 with spacing 8 produce wrong results with `current_tick / (spacing * 88)`.
**How to avoid:** Use `div_euclid` as already implemented in `src/protocols/orca.rs::tick_array_start_index()`. Do not rewrite this logic.
**Warning signs:** Tick array not found errors for positions with negative lower ticks.

### Pitfall 6: token_vault_a / _token_mint_a Behind Private Prefix

**What goes wrong:** Rust compilation error accessing `pool._token_vault_a` or similar — borsh positional fields have `_` prefix in the existing struct.
**Why it happens:** `WhirlpoolPool` uses `_` prefix convention for "layout-only" fields per CLAUDE.md / code comments. These are public fields in Rust but look private.
**How to avoid:** Access them directly as `pool._token_vault_a` (they ARE public, just prefixed for clarity). The `_` prefix in Rust is only a lint hint — it does not make fields private.
**Warning signs:** None — this compiles fine. Just needs awareness.

---

## Code Examples

### simulateTransaction Test Pattern

```rust
// Source: [VERIFIED: solana_client 1.18 RpcClient::simulate_transaction docs]
#[test]
#[ignore] // requires WALLET_KEYPAIR + RPC_URL env vars
fn simulate_close_position_ix() {
    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL required");
    let keypair_json = std::env::var("WALLET_KEYPAIR").expect("WALLET_KEYPAIR required");
    let bytes: Vec<u8> = serde_json::from_str(&keypair_json).unwrap();
    let wallet = solana_sdk::signer::keypair::Keypair::from_bytes(&bytes).unwrap();

    let rpc = solana_client::rpc_client::RpcClient::new(rpc_url);
    let blockhash = rpc.get_latest_blockhash().unwrap();

    // Build close_position instruction (with test pubkeys)
    let ix = build_close_position_ix(&wallet, &test_position_mint, &test_position_pda);
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(
        &[ix],
        Some(&wallet.pubkey()),
        &[&wallet],
        blockhash,
    );

    let result = rpc.simulate_transaction(&tx).unwrap();
    assert!(
        result.value.err.is_none(),
        "Simulation failed: {:?}\nLogs: {:#?}",
        result.value.err,
        result.value.logs,
    );
}
```

### Keypair Loading Guard

```rust
// Source: [VERIFIED: solana_sdk docs + anyhow error patterns from CLAUDE.md]
fn require_wallet_keypair() -> anyhow::Result<solana_sdk::signer::keypair::Keypair> {
    let raw = std::env::var("WALLET_KEYPAIR")
        .map_err(|_| anyhow::anyhow!(
            "WALLET_KEYPAIR env var not set; cannot start in --live mode.\n\
             Set it to a JSON byte array: export WALLET_KEYPAIR='[1,2,3,...]'"
        ))?;
    let bytes: Vec<u8> = serde_json::from_str(&raw)
        .context("WALLET_KEYPAIR must be a JSON byte array [u8; 64]")?;
    solana_sdk::signer::keypair::Keypair::from_bytes(&bytes)
        .map_err(|e| anyhow::anyhow!("Keypair::from_bytes failed: {}", e))
}
```

### Retry + Halt Pattern for open_position

```rust
// Source: [ASSUMED] — pattern from CONTEXT.md atomicity model
async fn open_with_retry(
    executor: &OrcaExecutor,
    args: &OpenPositionArgs,
) -> anyhow::Result<Signature> {
    match executor.open_position(args).await {
        Ok(sig) => Ok(sig),
        Err(e) => {
            tracing::warn!(error = %e, "open_position failed, retrying in 2s");
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            executor.open_position(args).await.map_err(|e2| {
                tracing::error!(
                    error = %e2,
                    "CRITICAL: open_position retry failed — position is CLOSED, no new position opened. Manual intervention required."
                );
                e2
            })
        }
    }
}
```

### Failure Row Persistence

```rust
// Source: existing storage::writer::ShadowRebalanceRow + CONTEXT.md error model
let fail_row = storage::writer::ShadowRebalanceRow {
    pool_address: pool_addr.clone(),
    trigger_reason: "live_rebalance_failed".to_string(),
    price: price_current,
    simulated_range_width: None,
    simulated_fees_earned: None,
    simulated_il_usd: None,
    simulated_net_pnl: None,
    error_flag: true,
    error_message: Some(format!("{:#}", err)),
};
storage::writer::spawn_shadow_write(pg.clone(), fail_row);
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `@orca-so/whirlpools` TypeScript SDK | `whirlpool-cpi` Rust crate (Anchor) or `orca_whirlpools_client` (Codama-generated) | 2023-2024 | Rust has mature CPI tools; no JS interop needed |
| Single combined transaction for rebalance | Three separate transactions (close, collect, open) | Ongoing | CU budget too tight for combined tx at current Solana limits |
| Anchor CPI from on-chain program | Off-chain `RpcClient::send_and_confirm_transaction` from binary | N/A | tick-liq is a binary, not an on-chain program |
| `whirlpool-cpi` old `main` branch | `anchor/0.29.0` branch | 2023 | Branch pinned to Anchor version; use branch not main |

**Deprecated/outdated:**
- `@orca-so/whirlpool-sdk` npm package: superseded by `@orca-so/whirlpools`; Rust equivalent is `orca_whirlpools_client` but that requires solana-program v3
- `orca_whirlpools_core` for instruction building: this crate is math-only (tick/price/IL math); cannot build Whirlpool instructions

---

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `whirlpool-cpi anchor/0.29.0` + `solana-sdk 1.18` will resolve without `[patch.crates-io]` because `orca_whirlpools_core` (the only Orca dep already present) has no `solana-program` dep | Cargo Version Conflict | If wrong: `cargo build` fails with version conflict; fix is `cargo update solana-program@<v3> --precise 1.18.17` as documented by Orca |
| A2 | `ToAccountMetas` and `InstructionData` are the correct trait names for building `solana_sdk::instruction::Instruction` from whirlpool-cpi account + instruction structs | Pattern 1 code example | If wrong: compilation error; look at `anchor_lang` 0.29 exported traits |
| A3 | Phase 5 positions are assumed to have zero liquidity before `close_position` is called (i.e., `decrease_liquidity` is out of scope or pre-conditions are met) | Pitfall 2 | If wrong: `close_position` fails with program error; need to add `decrease_liquidity` step |
| A4 | The `_token_vault_a`, `_token_vault_b`, `_token_mint_a`, `_token_mint_b` fields on `WhirlpoolPool` are Rust-public despite the `_` prefix | Collect Fees accounts | If wrong: cannot access vault/mint addresses from the existing pool struct; fix is to add accessor methods or change field names |
| A5 | `open_position` args are `tick_lower_index: i32, tick_upper_index: i32` with no bump argument in `whirlpool-cpi 0.1.4` | Accounts Reference | If wrong: look at the actual `whirlpool_cpi::instruction::OpenPosition` struct fields after adding the dep |

---

## Open Questions

1. **Does decrease_liquidity need to be in scope for Phase 5?**
   - What we know: `close_position` requires zero liquidity; the existing system accumulates liquidity in positions
   - What's unclear: Is the assumption that Phase 5 only fires on positions that have already been manually drained, or does Phase 5 need to call `decrease_liquidity` itself?
   - Recommendation: Planner should add a pre-flight check (`pos.liquidity == 0`) and halt with CRITICAL log if non-zero. Document that `decrease_liquidity` is a future task. A5 assumption in the assumptions log.

2. **Does `whirlpool-cpi` export instruction structs for off-chain use?**
   - What we know: The crate targets on-chain CPI but exports Anchor instruction types that include discriminators + account meta builders
   - What's unclear: Whether `anchor_lang::ToAccountMetas` works from a non-`#[program]` binary
   - Recommendation: After adding the dep, confirm with a `cargo check`. If the trait isn't available off-chain, fall back to hand-building `AccountMeta` vecs from the known account layouts (fully documented above).

3. **Is `OrcaExecutor` struct vs. free functions the right pattern?**
   - What we know: CONTEXT.md leaves this to Claude's discretion
   - Recommendation: Use an `OrcaExecutor { rpc_url: String, wallet: Arc<Keypair> }` struct — it encapsulates the connection and signing key, and its methods map 1:1 to the 4 instructions; testable by mocking the RPC URL in `#[ignore]` tests.

---

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| Rust/Cargo | Building whirlpool-cpi | Yes | cargo 1.94.1 / rustc 1.94.1 | — |
| Solana CLI (`solana`) | devnet keypair generation | No | — | Generate keypair via `Keypair::new()` in test + export as JSON |
| Anchor CLI | Build Anchor programs (not needed for client) | No | — | Not needed — tick-liq is not an on-chain program |
| `RPC_URL` (devnet/mainnet) | `#[ignore]` tests via simulateTransaction | Not verified | — | Set via env var at test time |
| `WALLET_KEYPAIR` env var | LIVE-03 + test suite | Not set in CI | — | Tests are `#[ignore]` by default; manual export required |
| PostgreSQL | `shadow_rebalances` failure rows | Not verified locally | — | Tests should mock or skip DB writes |

**Missing dependencies with no fallback:**
- `WALLET_KEYPAIR` and `RPC_URL` must be provided manually to run `#[ignore]` simulate tests — this is intentional per CONTEXT.md test strategy

**Missing dependencies with fallback:**
- Solana CLI not installed: not needed; keypairs generated in Rust code

---

## Project Constraints (from CLAUDE.md)

- Use `anyhow` for all error handling; no `unwrap()` in production paths
- Solana account deserialization: use `borsh` or the protocol's own crate; always verify program owner before deserializing
- Math must be validated against Orca Whirlpool JS SDK as reference
- Test math with `proptest` property-based tests
- Keypairs only via environment variables, never in config files or code
- `cargo clippy -- -D warnings` must pass

**Phase 5 implications:**
- All `Result` returns in `OrcaExecutor` must use `anyhow::Result`
- Instruction building errors must propagate with `.context()` chains
- `WALLET_KEYPAIR` env var is the only acceptable keypair source (aligns with LIVE-03)
- The `#[ignore]` test approach satisfies the "no keypairs in code" constraint

---

## Sources

### Primary (HIGH confidence)
- Orca Whirlpool program source, `close_position.rs` — [github.com/orca-so/whirlpools](https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/instructions/close_position.rs) — account struct verified
- Orca Whirlpool program source, `open_position.rs` — [github.com/orca-so/whirlpools](https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/instructions/open_position.rs) — account struct verified
- `whirlpool-cpi` Cargo.toml, `anchor/0.29.0` branch — [github.com/orca-so/whirlpool-cpi](https://github.com/orca-so/whirlpool-cpi) — deps: anchor-lang =0.29.0, cpi feature
- `orca_whirlpools_client` Cargo.toml — [raw.githubusercontent.com/orca-so/whirlpools](https://raw.githubusercontent.com/orca-so/whirlpools/main/rust-sdk/client/Cargo.toml) — requires solana-program ^3 (incompatible)
- `ClosePositionCpiAccounts` / `OpenPositionCpiAccounts` / `CollectFeesCpiAccounts` — [docs.rs/orca_whirlpools_client/7.2.0](https://docs.rs/orca_whirlpools_client/7.2.0/orca_whirlpools_client/) — account fields verified
- `RpcClient::simulate_transaction` — [docs.rs/solana-client/latest](https://docs.rs/solana-client/latest/solana_client/rpc_client/struct.RpcClient.html#method.simulate_transaction) — signature + return type verified
- `RpcSimulateTransactionResult` — [docs.rs/solana-client/latest](https://docs.rs/solana-client/latest/solana_client/rpc_response/struct.RpcSimulateTransactionResult.html) — err/logs/units_consumed fields

### Secondary (MEDIUM confidence)
- whirlpools-cpi-examples README — Anchor 0.29 + solana-program patching requirement — [github.com/orca-so/whirlpools-cpi-examples](https://github.com/orca-so/whirlpools-cpi-examples)
- `update_fees_and_rewards` accounts (tick_array_lower/upper required) — Orca program source, search-verified across multiple sources
- DEV.to article on fee collection ordering — [dev.to/tomtomdu73](https://dev.to/tomtomdu73/collecting-fees-and-rewards-from-orca-whirlpool-positions-5af5) — "must call update_fees_and_rewards before collecting"

### Tertiary (LOW confidence)
- Exact `ToAccountMetas` / `InstructionData` trait names in off-chain context — not verified; need `cargo check` after adding dep
- `decrease_liquidity` requirement in Phase 5 scope — inferred from program constraints; not explicitly tested

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — whirlpool-cpi crate verified, version conflict with orca_whirlpools_client confirmed
- Architecture (correct sequence): HIGH — update_fees_and_rewards → collect_fees requirement multi-source verified
- Accounts: HIGH — verified from Orca Whirlpool program source + CpiAccounts structs on docs.rs
- simulateTransaction API: HIGH — verified from solana-client docs.rs
- Cargo compatibility (no conflict): MEDIUM — likely safe but assumes orca_whirlpools_core is math-only

**Research date:** 2026-04-10
**Valid until:** 2026-05-10 (Orca Whirlpool program changes rarely; anchor 0.29 API is frozen)
