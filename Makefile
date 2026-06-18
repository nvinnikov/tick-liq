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
#   make run ARGS="position --mint <MINT>"
#   make run ARGS="watch --mint <MINT>"
#   make run ARGS="backtest --entry-price 84 --price-lower 75 --price-upper 95 --days 30 --rebalance"

run:
	@[ -f .env ] && export $$(grep -v '^#' .env | xargs) ; cargo run -- $(ARGS)

# ── Migrations ───────────────────────────────────────────────────────────────
# Schema is embedded and applied by the binary itself (storage::run_migrations);
# there is no sqlx migrations/ directory. `db migrate` is re-runnable.

migrate:
	@[ -f .env ] && export $$(grep -v '^#' .env | xargs) ; cargo run -- db migrate

# ── Housekeeping ─────────────────────────────────────────────────────────────

clean:
	cargo clean
	docker compose down -v
