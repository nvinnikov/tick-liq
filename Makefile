.PHONY: up down logs build test lint fmt run migrate clean

# ── DB infra ─────────────────────────────────────────────────────────────────

up:
	docker compose up -d
	@echo "Waiting for DB to be ready..."
	@docker compose exec db pg_isready -U tick -d tick_liq -q || sleep 3

down:
	docker compose down

logs:
	docker compose logs -f db

# ── Rust ─────────────────────────────────────────────────────────────────────

build:
	cargo build

test:
	cargo test

lint:
	cargo clippy -- -D warnings

fmt:
	cargo fmt

# ── Run ──────────────────────────────────────────────────────────────────────
# Usage:
#   make run ARGS="pool info --address <ADDR>"
#   make run ARGS="position monitor --mint <MINT>"
#   make run ARGS="backtest --pool <ADDR> --days 30 --strategy rebalance"

run:
	@[ -f .env ] && export $$(grep -v '^#' .env | xargs) ; cargo run -- $(ARGS)

# ── Migrations ───────────────────────────────────────────────────────────────

migrate:
	@[ -f .env ] && export $$(grep -v '^#' .env | xargs) ; \
	  sqlx migrate run || echo "sqlx not installed: cargo install sqlx-cli"

# ── Housekeeping ─────────────────────────────────────────────────────────────

clean:
	cargo clean
	docker compose down -v
