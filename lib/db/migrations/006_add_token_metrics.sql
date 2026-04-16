-- Migration: token_metrics table for historical price snapshot storage (Task #30)
-- Stores per-token price snapshots written by the Python strategy engine every 60s.
-- Used by POST /api/backtest to replay strategy logic against real historical data.
-- Safe to run multiple times (IF NOT EXISTS guards).

-- ── token_metrics table ──────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS token_metrics (
    id                    SERIAL PRIMARY KEY,
    mint                  TEXT NOT NULL,
    price                 DOUBLE PRECISION NOT NULL,
    liquidity_sol         DOUBLE PRECISION,
    market_cap_sol        DOUBLE PRECISION,
    volume_24h_sol        DOUBLE PRECISION,
    holder_count          INTEGER,
    bonding_curve_progress DOUBLE PRECISION,
    recorded_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── Composite index for per-mint time-series queries ─────────────────────────
CREATE INDEX IF NOT EXISTS token_metrics_mint_recorded_at_idx
  ON token_metrics (mint, recorded_at);

-- ── Index for global time-range queries (e.g. last N days across all tokens) ─
CREATE INDEX IF NOT EXISTS token_metrics_recorded_at_idx
  ON token_metrics (recorded_at);
