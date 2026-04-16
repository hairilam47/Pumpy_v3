-- Migration: Persistent idempotency keys for crash-safe deduplication (Task #41)
-- The Rust engine also creates this table via run_migrations() on startup.
-- Safe to run multiple times (IF NOT EXISTS guards).

-- ── idempotency_keys table ────────────────────────────────────────────────────
-- ikey:       The caller-supplied idempotency key (TEXT, arbitrary string).
-- order_id:   Empty string = "in-flight" reservation; non-empty = committed order.
-- created_at: Used for TTL-based cleanup (60-second window matches in-memory TTL).
CREATE TABLE IF NOT EXISTS idempotency_keys (
    ikey       TEXT PRIMARY KEY,
    order_id   TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── Index for efficient TTL cleanup ──────────────────────────────────────────
CREATE INDEX IF NOT EXISTS idempotency_keys_created_at_idx
  ON idempotency_keys (created_at);
