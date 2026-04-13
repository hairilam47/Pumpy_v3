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

    info!("Database migrations complete");
    Ok(())
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
