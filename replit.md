# PumpyPumpyFunBotTrade

## Overview

A Pump.fun Solana trading bot with four components:
1. **Rust Execution Engine** — Solana WebSocket monitoring, Jito MEV protection, tonic gRPC server (port 50051)
2. **Python ML Strategy Engine** — sniper/momentum strategies, Random Forest ML signals, FastAPI (port 8001)
3. **Express API Server** — REST + WebSocket bridge (port 8080)
4. **React Monitoring Dashboard** — real-time trade feed, portfolio metrics, strategy configurator

Source: https://github.com/hairilam47/Pumpy_v3

## Architecture

**Development**: Dashboard (port 23183, Vite dev) → Express API Server (port 8080) → Python FastAPI (port 8001) → Rust gRPC (port 50051). Each service runs in its own workflow.

**Production (Autoscale)**: Everything consolidates onto a single port (8080).
- Express serves the React dashboard's static build (`artifacts/dashboard/dist/public`) at `/dashboard/`.
- Express also serves `/api/*` routes and the WebSocket fan-out at `/api/bot/stream`.
- Bare `/` issues a 301 redirect to `/dashboard/`.
- The Python strategy engine is spawned as a child process by Express on startup when `NODE_ENV=production`. The interpreter defaults to `python3` (the deployment image does not provide a bare `python` symlink) and can be overridden with the `PYTHON_BIN` env var. The "started" log line only fires after the child's `spawn` event so a failed launch is no longer masked by a false success.
- The Rust gRPC engine remains a separate service and is reached via `RUST_GRPC_URL`.
- Run command: `pnpm --filter @workspace/api-server run start` (set in Deployments UI). Build command: `bash scripts/build-production.sh`. `deploymentTarget = "vm"` in `artifacts/api-server/.replit-artifact/artifact.toml`.

## Stack

- **Monorepo tool**: pnpm workspaces
- **Node.js version**: 24
- **Package manager**: pnpm
- **TypeScript version**: 5.9
- **API framework**: Express 5 (api-server)
- **Database**: PostgreSQL + Drizzle ORM (trades, strategies, bot_config, wallet_config tables)
- **Validation**: Zod, drizzle-zod
- **API codegen**: Orval (from `lib/api-spec/openapi.yaml`)
- **Build**: esbuild (CJS bundle for api-server), Vite (dashboard)
- **Rust**: tokio, tonic, solana-sdk, jito-searcher-client, async-stream
- **Python**: FastAPI, scikit-learn, joblib, grpcio, protobuf, tenacity, structlog

## Key Commands

- `pnpm run typecheck` — full typecheck across all packages
- `pnpm --filter @workspace/api-spec run codegen` — regenerate API hooks and Zod schemas from OpenAPI spec
- `pnpm --filter @workspace/db run push` — push DB schema changes
- `pnpm --filter @workspace/api-server run dev` — run API server locally
- `cargo check` (in rust-engine/) — typecheck Rust
- `python main.py` (in python-strategy/) — run Python strategy engine
- `bash docs/scripts/backup.sh` — manual database + ML model backup

## Rust Engine Features

- **Exponential backoff**: `process_order()` retries with `retry_delay * 2^(retry_count-1)`, up to 3 attempts
- **Dynamic slippage**: `compute_buy_params()` / `compute_sell_params()` on `BondingCurveParams`; applied_bps = `clamp(impact * 1.5, 50, max_slippage_bps)`
- **Dynamic Jito tip**: reads `JITO_TIP_PERCENT/FLOOR/CEILING` from bot_config; falls back to env vars
- **Pre-submission simulation**: `simulate_transaction()` in `PumpFunClient` validates txn before Jito submission
- **Jito retry pattern**: on bundle failure → wait 2s → retry once → fall back to RPC
- **Idempotency cache**: `BotService` holds an in-memory HashMap keyed by `idempotency_key` with 5-minute TTL; duplicate requests return the existing order_id
- **Distributed tracing**: `trace_id` logged on every `SubmitOrder` call and propagated through logs
- **gRPC proto fields**: `SubmitOrderRequest` fields 11 (`client_order_id`), 12 (`idempotency_key`), 13 (`trace_id`)

## Python Strategy Engine (`python-strategy/`)

- `main.py` — FastAPI entry point; lifespan hook starts/stops StrategyEngine
- `strategy_engine.py` — orchestrates strategies, data collector, Prometheus (port 9092); computes Sharpe ratio, max drawdown, volatility
- `config.py` — all settings via pydantic-settings (env vars)
- `grpc_client/client.py` — async gRPC client with **circuit breaker** (5-failure threshold, 30s recovery), UUID generation for `client_order_id` + `trace_id`
- `grpc_client/bot_pb2*.py` — generated proto stubs
- `analytics/data_collector.py` — PumpFunDataCollector: streams StreamOrders gRPC → token events
- `ml/signal_generator.py` — MLSignalGenerator: rule-based + scikit-learn RF model; **joblib persistence** (`.joblib` format, pickle fallback); `reload_if_stale()` for hot-reload
- `strategies/sniper.py` — PumpFunSniper: early bonding curve sniping strategy
- `strategies/momentum.py` — MomentumTrader: volume/price momentum strategy
- `api/routes.py` — FastAPI routes: /health, /metrics (with Sharpe/drawdown/volatility), /strategies, /portfolio, /orders, /tokens, /backtest (POST)

## Circuit Breaker (Python)

Three-state (CLOSED → OPEN → HALF_OPEN → CLOSED):
- Opens after 5 consecutive gRPC fatal errors
- Stays open for 30s, then enters HALF_OPEN for a single probe
- `circuit_breaker_state` returned in `/api/bot/metrics` and displayed on the dashboard

## WebSocket Fan-Out (API Server)

`artifacts/api-server/src/index.ts` maintains a **singleton gRPC StreamOrders call** at server startup. All WebSocket clients subscribe to an `EventEmitter` (`orderEmitter`) instead of opening their own gRPC connections. Each client can optionally filter by `order_id` sets. Auto-reconnects on gRPC stream failure.

**Keepalive**: Each connected WS client receives a ping every 20 s (`WS_PING_INTERVAL_MS`). Clients that miss a pong are terminated via `ws.terminate()`. This prevents the Replit proxy from silently dropping idle connections after its ~30 s inactivity timeout. The interval is cleared in the `ws.on("close")` handler to avoid leaks.

## Advanced Metrics (Task #30)

Python engine exposes per `/api/metrics`:
- `sharpe_ratio` — annualised Sharpe (288 trades/day × 365 days)
- `max_drawdown_sol` — peak-to-trough cumulative PnL
- `volatility_sol` — per-trade PnL standard deviation
- `circuit_breaker_state` — CLOSED / OPEN / HALF_OPEN

`POST /api/backtest` — replay-based simulation returning equity curve, Sharpe, drawdown, win rate.

## Dashboard Pages

1. **Dashboard** (`/`) — portfolio cards, 24h PnL chart, engine metrics, advanced strategy metrics (Sharpe, max drawdown, volatility, circuit breaker), MEV panel, live WebSocket trade feed, start/stop controls
2. **Strategies** (`/strategies`) — enable/disable sniper & momentum strategies, edit buy amount per strategy
3. **Wallets** (`/wallets`) — wallet management, pause/resume with admin key
4. **Tokens** (`/tokens`) — tracked token table with price, market cap, liquidity, holder count, ML score, bonding curve progress
5. **Trades** (`/trades`) — full trade history table, manual order form
6. **Settings** (`/settings`) — two-tier config: read-only wallet section (Replit Secrets), editable connection/trading/service URL sections (saved to DB)

## Dashboard Components

- `artifacts/dashboard/src/hooks/use-live-trades.ts` — WebSocket hook connecting to `/api/bot/stream`
- `artifacts/dashboard/src/hooks/use-admin-key.ts` — **admin key session cache** (1-hour TTL via sessionStorage)
- `artifacts/dashboard/src/components/LiveTradesFeed.tsx` — real-time trade table with status badges
- `artifacts/dashboard/src/components/MevStatsPanel.tsx` — Jito bundle stats panel
- `artifacts/dashboard/src/components/OfflineBanner.tsx` — **offline WS banner** (amber fixed banner when API server unreachable, auto-reconnects every 5 s on close). Probe URL is `wss://host/api/bot/stream` (no `BASE_URL` prefix — that path is mounted at the host root, not under `/dashboard/`). Server-side ping/pong keeps the connection alive, so there is no client-side periodic re-probe.

## Database Schema

`trades` table columns: id, walletId, mint, tokenName, tokenSymbol, side, amountSol, price, status, strategy, signature, pnlSol, slippageBps, createdAt, executedAt, **clientOrderId**, **traceId**

## Operational Docs (`docs/`)

- `docs/runbooks/halted-wallet.md` — diagnosing and resuming auto-paused wallets
- `docs/runbooks/jito-bundle-failures.md` — Jito bundle failure diagnosis and tip adjustment
- `docs/runbooks/key-rotation.md` — rotating admin key, wallet key, and DB credentials
- `docs/runbooks/backup-restore.md` — backup procedure, retention, and disaster recovery
- `docs/scripts/backup.sh` — automated PostgreSQL dump + ML model backup with 7-day retention

## Key Files

- `rust-engine/src/main.rs` — Rust engine entry point
- `rust-engine/src/grpc_server.rs` — gRPC server with idempotency cache + trace_id logging
- `rust-engine/src/database.rs` — DB helpers including `get_config_value()` for bot_config
- `rust-engine/proto/bot.proto` — gRPC proto definition (fields 1–13 in SubmitOrderRequest)
- `python-strategy/main.py` — Python FastAPI entry point
- `python-strategy/strategy_engine.py` — strategy orchestrator + advanced metrics
- `artifacts/api-server/src/routes/bot.ts` — Express API bridge routes
- `artifacts/api-server/src/index.ts` — singleton gRPC stream + WebSocket fan-out
- `artifacts/api-server/src/lib/grpc-client.ts` — gRPC client
- `artifacts/dashboard/src/App.tsx` — Dashboard app with routing + OfflineBanner
- `artifacts/dashboard/vite.config.ts` — Vite config (proxies /api to port 8080, ws: true)
- `lib/api-spec/openapi.yaml` — OpenAPI spec
- `lib/db/src/schema/trades.ts` — trades table (with clientOrderId, traceId)

## Settings Architecture (Two-Tier Config)

- **Tier 1 (Replit Secrets — read-only in UI)**: `WALLET_PRIVATE_KEY`, `KEYPAIR_PATH`, `DATABASE_URL`
- **Tier 2 (DB-backed — editable in Settings page)**: `SOLANA_RPC_URL`, `SOLANA_RPC_URLS`, `JITO_BUNDLE_URL`, `JITO_TIP_PERCENT`, `JITO_TIP_FLOOR`, `JITO_TIP_CEILING`, `MAX_POSITION_SIZE_SOL`, `STOP_LOSS_PERCENT`, `TAKE_PROFIT_PERCENT`, `RUST_GRPC_URL`, `PYTHON_STRATEGY_URL`
- DB values take precedence over env vars. Rust engine reads DB config at startup. A restart is required for Rust to pick up changes.

## Deployment Build

`artifacts/api-server/build.mjs` (esbuild) bundles the Express app into `dist/index.mjs`. Pure-JS gRPC packages — `@grpc/grpc-js`, `@grpc/proto-loader`, and `protobufjs` — must NOT appear in the externals list, otherwise the production run container crashes with `ERR_MODULE_NOT_FOUND` (the deployment image excludes `node_modules`). Only genuinely native modules (`*.node`, `sharp`, `bcrypt`, etc.) belong in externals. Bundle size is ~2.8 MB.

`scripts/build-production.sh` runs the API-server bundle build and the dashboard Vite build (with required `PORT` and `BASE_PATH` env vars) so both `dist` directories land in the deployment image. `.replitignore` keeps `node_modules`, `rust-engine/target/`, and other build caches out of the image, but explicitly does NOT exclude the two `dist` directories.

## Important Config

- Pump.fun program ID: `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`
- Rust OpenSSL: `.cargo/config.toml` sets OPENSSL_DIR to Nix store paths
- Dashboard proxies `/api/*` to API server on port 8080 via Vite dev server proxy (with ws: true for WebSocket)
- Python API URL: controlled by `PYTHON_API_URL` env var (default: `http://localhost:8001`)
- Proto file path: resolved via `process.cwd()/../../../rust-engine/proto/bot.proto` from the api-server package dir
- WebSocket: Dashboard connects to `wss://host/api/bot/stream` (routed by Replit proxy to port 8080)
- Admin key: `ADMIN_API_KEY` env var; cached in browser `sessionStorage` with 1-hour TTL

See the `pnpm-workspace` skill for workspace structure details.
