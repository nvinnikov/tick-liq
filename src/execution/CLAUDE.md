# Review rules — `src/execution/` (money path: STRICT)

Rebalance + hedge execution. These rules apply **in addition to** the root
`CLAUDE.md` "Code Review Guidelines". This path moves real funds — treat safety
defects as 🔴 Blocker.

## ShadowGuard is the single submission chokepoint

- Every path that could submit / send / sign a transaction MUST go through
  `ShadowGuard::submit`. 🔴 Flag any RPC submit, `send_transaction`, signing, or
  CPI dispatch that bypasses `ShadowGuard`.
- Default must be `Shadow` (blocks, returns `ShadowGuardError::Blocked`). `Live`
  may only be reached via explicit, intentional opt-in (e.g. `--live`). 🔴 Flag
  anything that defaults to `Live` or silently flips the guard.

## Phased rollout — no premature live execution

- Execution is staged: plan/dry-run types (`RebalancePlan`, `compute_hedge_size`,
  `print_*`) do **no** RPC calls, no tx construction, no signing. 🔴 Flag real
  submission, signing, or keypair use introduced into dry-run/plan code ahead of
  the phase that is meant to add it.

## Secrets & keypairs

- Keypairs/secrets come only from environment variables — never config files,
  never literals. 🔴 Flag a keypair/secret read from disk or hardcoded.
- 🔴 Flag a secret, private key, or full signed tx logged via `tracing` or
  printed. (Note `ShadowGuard::submit` logs the tx with `?tx` in debug — fine for
  dummy/dry-run types, but flag if a real signed transaction with key material
  could reach a log line.)

## Rebalance correctness

- New ranges must be aligned to `tick_spacing` and must **strictly contain** the
  current tick (`new_tick_lower <= current_tick < new_tick_upper`). 🔴 Flag
  alignment that can place the current price outside the new range.
- Tick arithmetic on `i32` must use saturating/checked ops (existing code uses
  `saturating_*` and `rem_euclid`). 🔴 Flag raw `+`/`-`/`*` on ticks that can
  overflow or misalign negative ticks (must floor toward −∞, i.e. `rem_euclid`).
- Preserve the close → collect → open ordering. 🔴 Flag a sequence that closes
  a position without first collecting fees, or reopens before close confirms.

## Atomicity & swap safety

- A rebalance that closes but fails to reopen must be detectable and recoverable,
  not silently swallowed. 🔴 Flag `Result` discarded with `let _ =`, `.ok()`, or
  an empty error arm on a submission step.
- Any swap / open that exchanges value needs a slippage / min-out guard. 🟡 Flag
  a swap or liquidity-add with no slippage bound.
