-- Migration: Add client_order_id to trades for idempotency & order tracking (Task #26)
-- Safe to run multiple times (IF NOT EXISTS guards).

-- ── Add client_order_id column to trades ─────────────────────────────────────
-- Use UUID type for type-safe, format-enforced client order IDs.
-- For existing installs that ran an earlier TEXT version, migration 004 handles the type change.
ALTER TABLE trades ADD COLUMN IF NOT EXISTS client_order_id UUID;

-- ── Index for querying trades by client_order_id ──────────────────────────────
CREATE INDEX IF NOT EXISTS trades_client_order_id_idx
  ON trades (client_order_id)
  WHERE client_order_id IS NOT NULL;
