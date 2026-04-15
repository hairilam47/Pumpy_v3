-- Migration: Configuration Layer Discipline + Wallet Registry
-- Adds Layer B (system_config), Layer C (wallet_config), and wallet_registry tables.
-- Safe to run multiple times (IF NOT EXISTS / ON CONFLICT DO NOTHING).

-- ── Layer B: System Config (operator-only) ────────────────────────────────────
CREATE TABLE IF NOT EXISTS system_config (
  key         TEXT PRIMARY KEY,
  value       TEXT        NOT NULL,
  version     INTEGER     NOT NULL DEFAULT 1,
  description TEXT,
  updated_by  TEXT,
  updated_at  TIMESTAMPTZ DEFAULT NOW()
);

-- ── Layer C: Wallet Config (per-wallet, client-editable) ─────────────────────
CREATE TABLE IF NOT EXISTS wallet_config (
  wallet_id          TEXT PRIMARY KEY,
  risk_per_trade_sol DOUBLE PRECISION NOT NULL DEFAULT 0.1,
  daily_loss_limit_sol DOUBLE PRECISION NOT NULL DEFAULT 1.0,
  strategy_preset    TEXT NOT NULL DEFAULT 'balanced'
    CONSTRAINT wallet_config_strategy_preset_check
      CHECK (strategy_preset IN ('conservative', 'balanced', 'aggressive')),
  status             TEXT NOT NULL DEFAULT 'enabled'
    CONSTRAINT wallet_config_status_check
      CHECK (status IN ('enabled', 'paused', 'halted')),
  owner_pubkey       TEXT,
  created_at         TIMESTAMPTZ DEFAULT NOW(),
  updated_at         TIMESTAMPTZ DEFAULT NOW()
);

-- ── Wallet Registry ───────────────────────────────────────────────────────────
-- keypair_path is NEVER returned to the API or UI — backend-only.
CREATE TABLE IF NOT EXISTS wallet_registry (
  wallet_id      TEXT PRIMARY KEY,
  keypair_path   TEXT,        -- backend-only; omitted from all API responses
  status         TEXT NOT NULL DEFAULT 'enabled',
  owner_pubkey   TEXT,
  last_active_at TIMESTAMPTZ,
  created_at     TIMESTAMPTZ DEFAULT NOW()
);

-- ── System Config seed (idempotent) ──────────────────────────────────────────
INSERT INTO system_config (key, value, version, description, updated_by, updated_at) VALUES
  ('max_position_size_sol',      '1.0',   1, 'Maximum SOL per individual trade position',                           'system:seed', NOW()),
  ('max_portfolio_exposure_sol', '5.0',   1, 'Maximum total SOL exposure across all open positions',                'system:seed', NOW()),
  ('max_daily_loss_sol',         '2.0',   1, 'Maximum SOL loss allowed in a 24-hour window before halting',         'system:seed', NOW()),
  ('max_slippage_bps',           '1000',  1, 'Maximum allowed slippage in basis points (1000 = 10%)',               'system:seed', NOW()),
  ('max_sandwich_risk_score',    '70',    1, 'Maximum sandwich attack risk score (0-100) before rejecting trade',   'system:seed', NOW()),
  ('jito_tip_lamports',          '10000', 1, 'Default Jito bundle tip in lamports',                                 'system:seed', NOW()),
  ('jito_bundle_timeout_ms',     '5000',  1, 'Jito bundle submission timeout in milliseconds',                      'system:seed', NOW()),
  ('mev_protection_enabled',     'true',  1, 'Whether MEV protection via Jito is enabled by default',               'system:seed', NOW()),
  ('min_liquidity_sol',          '5.0',   1, 'Minimum liquidity (SOL) required for a token to be sniped',           'system:seed', NOW()),
  ('min_market_cap_usd',         '1000',  1, 'Minimum market cap USD for token discovery filter',                   'system:seed', NOW()),
  ('max_token_age_seconds',      '30',    1, 'Maximum age in seconds for auto-snipe eligibility',                   'system:seed', NOW()),
  ('strategy_sniper_enabled',    'true',  1, 'Whether the sniper strategy is available for activation',             'system:seed', NOW()),
  ('strategy_momentum_enabled',  'true',  1, 'Whether the momentum strategy is available for activation',           'system:seed', NOW()),
  ('strategy_scalp_enabled',     'false', 1, 'Whether the scalp strategy is available for activation',              'system:seed', NOW()),
  ('default_snipe_amount_sol',   '0.1',   1, 'Default SOL amount for auto-snipe orders',                           'system:seed', NOW()),
  ('max_retries',                '3',     1, 'Maximum order retry attempts before marking as failed',               'system:seed', NOW()),
  ('retry_delay_ms',             '1000',  1, 'Delay in milliseconds between order retries',                         'system:seed', NOW())
ON CONFLICT (key) DO NOTHING;
