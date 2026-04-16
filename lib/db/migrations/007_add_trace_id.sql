-- Migration 006: Add trace_id to trades table for distributed log correlation (Task #31)
ALTER TABLE trades ADD COLUMN IF NOT EXISTS trace_id TEXT;

CREATE INDEX IF NOT EXISTS trades_trace_id_idx ON trades (trace_id) WHERE trace_id IS NOT NULL;
