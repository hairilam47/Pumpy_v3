-- Migration: Add DB indexes and missing columns for query performance
-- Adds wallet_id to trades, created_at to wallet_alerts, and composite indexes.
-- Safe to run multiple times (IF NOT EXISTS / DO NOTHING guards).

-- ── Add wallet_id column to trades ───────────────────────────────────────────
ALTER TABLE trades ADD COLUMN IF NOT EXISTS wallet_id TEXT;

-- ── Indexes on trades ────────────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS trades_wallet_id_created_at_idx
  ON trades (wallet_id, created_at);

CREATE INDEX IF NOT EXISTS trades_mint_created_at_idx
  ON trades (mint, created_at);

-- ── Add created_at column to wallet_alerts ───────────────────────────────────
ALTER TABLE wallet_alerts ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ DEFAULT NOW();

-- ── Index on wallet_alerts ───────────────────────────────────────────────────
CREATE INDEX IF NOT EXISTS wallet_alerts_wallet_id_created_at_idx
  ON wallet_alerts (wallet_id, created_at);
