use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use std::time::Duration;
use tracing::info;

pub type DbPool = Pool<Postgres>;

#[derive(Clone)]
pub struct DatabasePool {
    pub pool: DbPool,
}

impl DatabasePool {
    pub async fn new(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .min_connections(2)
            .acquire_timeout(Duration::from_secs(10))
            .connect(database_url)
            .await?;

        info!("Database pool created");
        Ok(Self { pool })
    }
}

pub async fn run_migrations(db: &DatabasePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS orders (
            id TEXT PRIMARY KEY,
            mint TEXT NOT NULL,
            order_type TEXT NOT NULL,
            side TEXT NOT NULL,
            amount BIGINT NOT NULL,
            price DOUBLE PRECISION,
            max_cost BIGINT,
            min_output BIGINT,
            slippage_bps INTEGER NOT NULL,
            status TEXT NOT NULL,
            strategy TEXT NOT NULL DEFAULT '',
            metadata JSONB NOT NULL DEFAULT '{}',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            executed_at TIMESTAMPTZ,
            signature TEXT,
            error TEXT,
            retry_count INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(&db.pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS positions (
            id TEXT PRIMARY KEY,
            mint TEXT NOT NULL,
            amount BIGINT NOT NULL,
            entry_price DOUBLE PRECISION NOT NULL,
            current_price DOUBLE PRECISION NOT NULL DEFAULT 0,
            pnl DOUBLE PRECISION NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'OPEN',
            strategy TEXT NOT NULL DEFAULT '',
            opened_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            closed_at TIMESTAMPTZ
        )
        "#,
    )
    .execute(&db.pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS trades (
            id TEXT PRIMARY KEY,
            mint TEXT NOT NULL,
            side TEXT NOT NULL,
            amount BIGINT NOT NULL,
            price DOUBLE PRECISION NOT NULL,
            pnl DOUBLE PRECISION,
            signature TEXT,
            strategy TEXT NOT NULL DEFAULT '',
            executed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(&db.pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS bot_config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
    )
    .execute(&db.pool)
    .await?;

    // Layer B: operator-owned system configuration
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS system_config (
            key         TEXT PRIMARY KEY,
            value       TEXT        NOT NULL,
            version     INTEGER     NOT NULL DEFAULT 1,
            description TEXT,
            updated_by  TEXT,
            updated_at  TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(&db.pool)
    .await?;

    // Layer C: per-wallet client-editable configuration
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS wallet_config (
            wallet_id            TEXT PRIMARY KEY,
            risk_per_trade_sol   DOUBLE PRECISION NOT NULL DEFAULT 0.1,
            daily_loss_limit_sol DOUBLE PRECISION NOT NULL DEFAULT 1.0,
            strategy_preset      TEXT NOT NULL DEFAULT 'balanced'
                CONSTRAINT wallet_config_strategy_preset_check
                    CHECK (strategy_preset IN ('conservative', 'balanced', 'aggressive')),
            status               TEXT NOT NULL DEFAULT 'enabled'
                CONSTRAINT wallet_config_status_check
                    CHECK (status IN ('enabled', 'paused', 'halted')),
            owner_pubkey         TEXT,
            created_at           TIMESTAMPTZ DEFAULT NOW(),
            updated_at           TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(&db.pool)
    .await?;

    // Wallet registry — keypair_path is backend-only, never sent to clients
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS wallet_registry (
            wallet_id      TEXT PRIMARY KEY,
            keypair_path   TEXT,
            status         TEXT NOT NULL DEFAULT 'enabled',
            owner_pubkey   TEXT,
            last_active_at TIMESTAMPTZ,
            created_at     TIMESTAMPTZ DEFAULT NOW()
        )
        "#,
    )
    .execute(&db.pool)
    .await?;

    info!("Database migrations complete");
    Ok(())
}

#[derive(sqlx::FromRow)]
pub struct ConfigRow {
    pub key: String,
    pub value: String,
}

/// Row returned for public wallet listing (keypair_path intentionally excluded).
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct WalletRegistryRow {
    pub wallet_id: String,
    pub status: String,
    pub owner_pubkey: Option<String>,
}

/// Internal-only row that includes the keypair path.
/// MUST NOT be returned to any API or UI layer.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct WalletRegistryFullRow {
    pub wallet_id: String,
    pub keypair_path: Option<String>,
    pub status: String,
    pub owner_pubkey: Option<String>,
}

/// Load non-sensitive runtime config from the bot_config table.
/// Returns an empty map on any error — callers must fall back to env vars.
pub async fn load_db_config(pool: &DbPool) -> std::collections::HashMap<String, String> {
    match sqlx::query_as::<_, ConfigRow>("SELECT key, value FROM bot_config")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows.into_iter().map(|r| (r.key, r.value)).collect(),
        Err(e) => {
            tracing::warn!("Could not load bot_config from DB (using env vars only): {}", e);
            std::collections::HashMap::new()
        }
    }
}

/// Load all wallets from the registry for startup logging and multi-wallet orchestration.
/// Returns public rows only — keypair_path is NOT included.
pub async fn load_wallet_registry(pool: &DbPool) -> Vec<WalletRegistryRow> {
    match sqlx::query_as::<_, WalletRegistryRow>(
        "SELECT wallet_id, status, owner_pubkey FROM wallet_registry ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("Could not load wallet_registry from DB: {}", e);
            Vec::new()
        }
    }
}

/// Internal helper: load the first enabled wallet's keypair_path from the registry.
/// Used as a fallback when no env var wallet is configured (demo-mode override).
/// The returned path is never logged or sent over any API boundary.
pub async fn load_first_registry_keypair_path(pool: &DbPool) -> Option<String> {
    match sqlx::query_as::<_, WalletRegistryFullRow>(
        "SELECT wallet_id, keypair_path, status, owner_pubkey FROM wallet_registry \
         WHERE status = 'enabled' AND keypair_path IS NOT NULL \
         ORDER BY created_at \
         LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    {
        Ok(Some(row)) => {
            tracing::info!(
                wallet_id = %row.wallet_id,
                "wallet_registry: found enabled wallet, loading keypair from registered path"
            );
            row.keypair_path
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!("Could not query wallet_registry for keypair path: {}", e);
            None
        }
    }
}

pub async fn cleanup_old_data(db: &DbPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM orders WHERE created_at < NOW() - INTERVAL '30 days' AND status IN ('Executed', 'Failed', 'Cancelled', 'Expired')"
    )
    .execute(db)
    .await?;

    sqlx::query(
        "DELETE FROM trades WHERE executed_at < NOW() - INTERVAL '90 days'"
    )
    .execute(db)
    .await?;

    Ok(())
}
