# PumpyPumpyFunBotTrade

## Overview

A Pump.fun Solana trading bot with three components:
1. **Rust Execution Engine** — Solana WebSocket monitoring, Jito MEV protection, tonic gRPC server (port 50051)
2. **Python ML Strategy Engine** — sniper/momentum strategies, Random Forest ML signals, FastAPI (port 8001)
3. **React Monitoring Dashboard** — real-time trade feed, portfolio metrics, strategy configurator

Source: https://github.com/hairilam47/PumpyPumpyFunBotTrade

## Architecture

Dashboard (port 23183, Vite) → Express API Server (port 8080) → Python FastAPI (port 8001) → Rust gRPC (port 50051)

## Stack

- **Monorepo tool**: pnpm workspaces
- **Node.js version**: 24
- **Package manager**: pnpm
- **TypeScript version**: 5.9
- **API framework**: Express 5 (api-server)
- **Database**: PostgreSQL + Drizzle ORM (trades, strategies, bot_config tables)
- **Validation**: Zod, drizzle-zod
- **API codegen**: Orval (from `lib/api-spec/openapi.yaml`)
- **Build**: esbuild (CJS bundle for api-server), Vite (dashboard)
- **Rust**: tokio, tonic, solana-sdk, jito-searcher-client
- **Python**: FastAPI, scikit-learn, grpcio, protobuf

## Key Commands

- `pnpm run typecheck` — full typecheck across all packages
- `pnpm --filter @workspace/api-spec run codegen` — regenerate API hooks and Zod schemas from OpenAPI spec
- `pnpm --filter @workspace/db run push` — push DB schema changes
- `pnpm --filter @workspace/api-server run dev` — run API server locally
- `cargo check` (in rust-engine/) — typecheck Rust
- `python main.py` (in python-strategy/) — run Python strategy engine

## Python Strategy Engine (`python-strategy/`)

- `main.py` — FastAPI entry point; lifespan hook starts/stops StrategyEngine
- `strategy_engine.py` — orchestrates strategies, data collector, Prometheus (port 9092)
- `config.py` — all settings via pydantic-settings (env vars)
- `grpc_client/client.py` — async gRPC client connecting to Rust engine port 50051
- `grpc_client/bot_pb2*.py` — generated proto stubs (fixed import for package context)
- `analytics/data_collector.py` — PumpFunDataCollector: streams StreamOrders gRPC → token events
- `ml/signal_generator.py` — MLSignalGenerator: rule-based + scikit-learn RF model scoring
- `strategies/sniper.py` — PumpFunSniper: early bonding curve sniping strategy
- `strategies/momentum.py` — MomentumTrader: volume/price momentum strategy
- `api/routes.py` — FastAPI routes: /health, /metrics, /strategies, /portfolio, /orders, /tokens, /strategy/activate, /strategy/config

## Dashboard Pages

1. **Dashboard** (`/`) — portfolio cards, 24h PnL chart, engine metrics, MEV panel, live WebSocket trade feed, start/stop controls
2. **Strategies** (`/strategies`) — enable/disable sniper & momentum strategies, edit buy amount per strategy
3. **Tokens** (`/tokens`) — tracked token table with price, market cap, liquidity, holder count, ML score, bonding curve progress
4. **Trades** (`/trades`) — full trade history table, manual order form
5. **Settings** (`/settings`) — two-tier config: read-only wallet section (Replit Secrets), editable connection/trading/service URL sections (saved to DB)

## Dashboard Components

- `artifacts/dashboard/src/hooks/use-live-trades.ts` — WebSocket hook connecting to `/api/bot/stream`
- `artifacts/dashboard/src/components/LiveTradesFeed.tsx` — real-time trade table with status badges
- `artifacts/dashboard/src/components/MevStatsPanel.tsx` — Jito bundle stats panel

## Key Files

- `rust-engine/src/main.rs` — Rust engine entry point
- `rust-engine/src/grpc_server.rs` — gRPC server
- `rust-engine/proto/bot.proto` — gRPC proto definition (used by API server and Python stubs)
- `python-strategy/main.py` — Python FastAPI entry point
- `python-strategy/strategy_engine.py` — strategy orchestrator
- `artifacts/api-server/src/routes/bot.ts` — Express API bridge routes (includes /bot/mev-stats, /bot/start, /bot/stop)
- `artifacts/api-server/src/index.ts` — WebSocket server bridging gRPC to browser
- `artifacts/api-server/src/lib/grpc-client.ts` — gRPC client (proto path: process.cwd()/../../../rust-engine/proto/bot.proto)
- `artifacts/dashboard/src/App.tsx` — Dashboard app with routing
- `artifacts/dashboard/vite.config.ts` — Vite config (proxies /api to port 8080, ws: true)
- `lib/api-spec/openapi.yaml` — OpenAPI spec (10 bot endpoints)
- `lib/db/src/schema/trades.ts` — trades table
- `lib/db/src/schema/strategies.ts` — strategies table
- `lib/db/src/schema/bot-config.ts` — bot_config key-value table for runtime config

## Settings Architecture (Two-Tier Config)

- **Tier 1 (Replit Secrets — read-only in UI)**: `WALLET_PRIVATE_KEY`, `KEYPAIR_PATH`, `DATABASE_URL`
- **Tier 2 (DB-backed — editable in Settings page)**: `SOLANA_RPC_URL`, `SOLANA_RPC_URLS`, `JITO_BUNDLE_URL`, `MAX_POSITION_SIZE_SOL`, `STOP_LOSS_PERCENT`, `TAKE_PROFIT_PERCENT`, `RUST_GRPC_URL`, `PYTHON_STRATEGY_URL`
- DB values take precedence over env vars. Rust engine reads DB config at startup (after `run_migrations`). A restart is required for Rust to pick up changes.
- Endpoints: `GET /api/settings/config`, `PUT /api/settings/config`, `POST /api/settings/config/test-rpc`

## Important Config

- Pump.fun program ID: `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`
- Rust OpenSSL: `.cargo/config.toml` sets OPENSSL_DIR to Nix store paths
- Dashboard proxies `/api/*` to API server on port 8080 via Vite dev server proxy (with ws: true for WebSocket)
- Python API URL: controlled by `PYTHON_API_URL` env var (default: `http://localhost:8001`)
- Proto file path: resolved via `process.cwd()/../../../rust-engine/proto/bot.proto` from the api-server package dir
- WebSocket: Dashboard connects to `wss://host/api/bot/stream` (routed by Replit proxy to port 8080)

See the `pnpm-workspace` skill for workspace structure details.
