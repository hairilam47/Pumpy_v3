-- Migration: Convert trades.client_order_id from TEXT → UUID (Task #26 follow-up)
-- Applies to databases that ran migration 003 with a TEXT column.
-- All existing values are NULL so the USING cast is safe.
-- Safe to run multiple times: the ALTER is skipped if the column is already UUID.

DO $$
BEGIN
  IF EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'trades'
      AND column_name = 'client_order_id'
      AND data_type = 'text'
  ) THEN
    ALTER TABLE trades
      ALTER COLUMN client_order_id TYPE UUID USING client_order_id::UUID;
  END IF;
END
$$;
