---
phase: 05
slug: live-execution
status: verified
threats_open: 0
asvs_level: 1
created: 2026-04-10
---

# Phase 05 — Security

> Per-phase security contract: threat register, accepted risks, and audit trail.

---

## Trust Boundaries

| Boundary | Description | Data Crossing |
|----------|-------------|---------------|
| env→process | WALLET_KEYPAIR env var carries the signing keypair | ed25519 secret bytes — highest risk |
| process→Solana RPC | Signed transactions submitted to devnet/mainnet validator | Serialized transactions, signatures |
| whirlpool-cpi git dep | External git dependency pulled at build time | Compiled CPI discriminators / account structs |
| on_notify closure→OrcaExecutor | Async event callback dispatches real RPC calls | Pool state, plan parameters |

---

## Threat Register

| Threat ID | Category | Component | Disposition | Mitigation | Status |
|-----------|----------|-----------|-------------|------------|--------|
| T-05-01 | Tampering | whirlpool-cpi git dep | mitigate | Cargo.toml:52 pins `branch = "anchor/0.29.0"`; Cargo.lock commits exact hash `3d6587a86ab2fabaf157655cc8442a1a3703f368` | closed |
| T-05-02 | Tampering | Instruction account ordering | mitigate | Accounts hard-coded in orca_executor.rs:61-66,94-104,125-132,169-180; unit tests at lines 311-357 assert counts 4/9/6/10 | closed |
| T-05-03 | Information Disclosure | keypair in memory | accept | See Accepted Risks Log | closed |
| T-05-04 | Denial of Service | bad Cargo dep version | mitigate | Cargo.toml:53-54 pins `anchor-lang = "=0.29.0"` and `anchor-spl = "=0.29.0"` with exact-version constraint | closed |
| T-05-05 | Spoofing | new position_mint identity | mitigate | orca_executor.rs:159 `Keypair::new()`; line 173 new mint listed as signer; line 234 `submit_tx_with_extra_signer` includes mint as co-signer | closed |
| T-05-06 | Spoofing | WALLET_KEYPAIR env var | mitigate | main.rs:221 `Keypair::from_bytes()` validates ed25519 key; lines 491-493 `process::exit(1)` on parse failure — no silent fallback | closed |
| T-05-07 | Information Disclosure | Keypair in process memory | accept | See Accepted Risks Log | closed |
| T-05-08 | Tampering | Retry on open_position failure | mitigate | main.rs:843 2s delay before retry; lines 845-848 fresh `Keypair::new()` per attempt; parameters not mutated between attempts | closed |
| T-05-09 | Denial of Service | Failure row writes blocking tick loop | mitigate | storage/writer.rs:171-172 `tokio::spawn(async move {...})` fire-and-forget; caller never awaits — tick loop unblocked | closed |
| T-05-10 | Elevation of Privilege | open_position on wrong pool | mitigate | main.rs:511 pool_pubkey derived from on-chain `pos.whirlpool`; lines 832-833 tick bounds from `plan.new_tick_lower/upper` (build_rebalance_plan output) | closed |
| T-05-11 | Repudiation | Live rebalance outcome not logged | mitigate | main.rs:830-834 `tracing::info!(signature = %sig)` on success; lines 872-888 `spawn_shadow_write` with `error_flag: true` on failure — both outcomes auditable | closed |
| T-05-12 | Information Disclosure | simulateTransaction test logs | accept | See Accepted Risks Log | closed |

*Status: open · closed*
*Disposition: mitigate (implementation required) · accept (documented risk) · transfer (third-party)*

---

## Accepted Risks Log

| Risk ID | Threat Ref | Rationale | Accepted By | Date |
|---------|------------|-----------|-------------|------|
| AR-05-01 | T-05-03 | Keypair lives in Arc<Keypair> (no copies); this plan does not load the keypair — it is wired in Plan 05-02 only; RAM-only, no persistence | plan author | 2026-04-10 |
| AR-05-02 | T-05-07 | Keypair in Arc<Keypair> — no copies made; never logged (tracing emits pubkey only, never secret bytes); env var read once at startup and not retained | plan author | 2026-04-10 |
| AR-05-03 | T-05-12 | simulateTransaction tests gated behind `#[ignore]` — only run by explicit operator choice; no secrets appear in test output (keypair consumed for signing, not printed) | plan author | 2026-04-10 |

---

## Security Audit Trail

| Audit Date | Threats Total | Closed | Open | Run By |
|------------|---------------|--------|------|--------|
| 2026-04-10 | 12 | 12 | 0 | gsd-security-auditor (sonnet) |

---

## Sign-Off

- [x] All threats have a disposition (mitigate / accept / transfer)
- [x] Accepted risks documented in Accepted Risks Log
- [x] `threats_open: 0` confirmed
- [x] `status: verified` set in frontmatter

**Approval:** verified 2026-04-10
