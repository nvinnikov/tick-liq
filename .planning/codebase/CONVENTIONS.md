# CONVENTIONS.md — Code Conventions

## Error Handling

- Use `anyhow::Result<T>` as return type everywhere outside math (which may use plain `f64` / `Result<T, MyError>`)
- Propagate with `?` — no `.unwrap()` or `.expect()` in non-test code
- Add context with `.context("what we were doing")`
- Custom error types only when callers need to match on error variants

## Logging

- Use `tracing` macros: `tracing::info!`, `tracing::warn!`, `tracing::error!`, `tracing::debug!`
- Structured fields: `tracing::info!(pool = %address, tick = current_tick, "pool state updated")`
- Initialize subscriber in `main.rs` only
- Never log keypair material or secrets

## Async

- All I/O is async (`tokio`)
- `#[tokio::main]` only in `main.rs`
- Spawn tasks with `tokio::spawn` for concurrent monitoring loops
- Use `tokio::sync::mpsc` for inter-task communication

## Code Organization

- One concept per file; keep files focused
- Public API exposed via `mod.rs` re-exports
- Math functions are pure — no side effects, no I/O, no panics on valid input
- Strategy layer takes data structs as input, returns signal structs (no direct execution)
- Execution layer is the only place that submits transactions

## Configuration

- All external config loaded at startup into a typed `Config` struct (`src/config.rs`)
- Secrets (keypairs, RPC keys) via environment variables, never in `config.toml`
- Use `config.toml.example` to document all fields

## Safety Rules (Solana-specific)

- Always check `account.owner == expected_program_id` before deserializing account data
- Use connection pool for RPC — never create a new client per request
- WebSocket connections must have reconnect logic with exponential backoff
- Dry-run mode must be tested before mainnet execution

## Style

- `cargo fmt` enforced — no manual formatting decisions
- `cargo clippy -- -D warnings` must pass
- Inline doc comments (`///`) on all public types and functions
- Module-level doc comment (`//!`) in each `mod.rs`