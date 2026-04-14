use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tonic::transport::Server;
use tracing::{info, error};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod config;
mod constants;
mod database;
mod grpc_server;
mod mev;
mod metrics;
mod order;
mod pumpfun;
mod rpc;
mod transaction;
mod websocket;

use config::Config;
use database::DatabasePool;
use grpc_server::{BotService, bot_proto::bot_server::BotServer};
use mev::MevProtector;
use metrics::Metrics;
use order::{OrderManager, manager::OrderManagerConfig};
use pumpfun::PumpFunClient;
use rpc::RpcManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    // Initialize structured logging
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .json()
                .with_current_span(true),
        )
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    info!("Starting PumpFun Trading Engine v{}", env!("CARGO_PKG_VERSION"));

    // Load configuration from environment
    let mut config = Config::from_env().map_err(|e| format!("Config error: {}", e))?;
    info!("Environment: {}", config.environment);
    info!("gRPC port: {}", config.grpc_port);
    info!("Metrics port: {}", config.metrics_port);

    // Initialize Prometheus metrics
    let metrics = Arc::new(Metrics::new().map_err(|e| format!("Metrics error: {}", e))?);

    // Start Prometheus metrics server
    {
        let m = metrics.clone();
        let port = config.metrics_port;
        tokio::spawn(async move {
            m.start_server(port).await;
        });
    }

    // Initialize database first so DB-backed config overrides are applied BEFORE
    // RpcManager is constructed with the (potentially overridden) endpoint list.
    let db_pool = match DatabasePool::new(&config.database_url).await {
        Ok(pool) => {
            if let Err(e) = database::run_migrations(&pool).await {
                error!("Migration warning: {}", e);
            }
            // Load runtime overrides from bot_config (best-effort, no crash).
            // MUST happen before RpcManager so DB SOLANA_RPC_URL/URLS take effect.
            let db_overrides = database::load_db_config(&pool.pool).await;
            if !db_overrides.is_empty() {
                info!("Applying {} runtime override(s) from bot_config", db_overrides.len());
                config.apply_db_overrides(&db_overrides);
            }
            info!("Database connected");
            pool
        }
        Err(e) => {
            error!("Database connection failed (running without persistence): {}", e);
            // Create a pool with a dummy URL to allow graceful degradation
            DatabasePool::new("postgresql://localhost:5432/pumpfun?connect_timeout=1")
                .await
                .unwrap_or_else(|_| {
                    panic!("Cannot initialize database connection pool");
                })
        }
    };

    // Initialize multi-RPC manager using the final resolved endpoint list
    // (env vars + DB overrides already applied above).
    let rpc_manager = Arc::new(
        RpcManager::new(config.rpc_endpoints.clone())
            .await
            .map_err(|e| format!("RPC manager error: {}", e))?,
    );
    let rpc_manager = Arc::new(rpc_manager.as_ref().clone().with_metrics(metrics.clone()));
    rpc_manager.start_health_checks();
    info!("RPC manager initialized with {} endpoints", config.rpc_endpoints.len());

    // Initialize PumpFun client
    let pumpfun_client = Arc::new(
        PumpFunClient::new(rpc_manager.clone(), config.keypair_bytes.clone())
            .map_err(|e| format!("PumpFun client error: {}", e))?,
    );
    info!("PumpFun client initialized: wallet={}", pumpfun_client.pubkey());

    // Initialize MEV protector + standalone JitoClient for OrderManager
    let jito_client_opt: Option<Arc<crate::mev::JitoClient>> = config
        .jito_bundle_url
        .as_ref()
        .map(|url| Arc::new(crate::mev::JitoClient::new(url.clone())));

    let mev_protector = Arc::new(MevProtector::new(
        config.jito_bundle_url.clone(),
        pumpfun_client.clone(),
        metrics.clone(),
        config.risk_limits.max_sandwich_risk_score,
        config.trading.mev_protection_enabled,
    ));
    info!("MEV protector initialized (Jito: {})", mev_protector.has_jito());

    // Initialize order manager
    let order_config = OrderManagerConfig {
        max_pending_orders: 100,
        order_timeout: Duration::from_secs(config.order_timeout_seconds),
        max_retries: config.trading.retry_attempts,
        retry_delay: Duration::from_millis(config.trading.retry_delay_ms),
        max_position_size_sol: config.risk_limits.max_position_size_sol,
        max_portfolio_exposure_sol: config.risk_limits.max_portfolio_exposure_sol,
        max_daily_loss_sol: config.risk_limits.max_daily_loss_sol,
        max_sandwich_risk_score: config.risk_limits.max_sandwich_risk_score,
    };

    let order_manager = Arc::new(OrderManager::new(
        db_pool.clone(),
        pumpfun_client.clone(),
        mev_protector.clone(),
        jito_client_opt,
        metrics.clone(),
        order_config,
        config.trading.mev_protection_enabled,
    ));
    info!("Order manager initialized");

    // Start execution workers
    let semaphore = Arc::new(Semaphore::new(config.execution_workers));
    for i in 0..config.execution_workers {
        let manager = order_manager.clone();
        let sem = semaphore.clone();
        tokio::spawn(async move {
            info!("Execution worker {} started", i);
            manager.execution_worker(sem).await;
        });
    }

    // Start WebSocket monitor (logsSubscribe + programSubscribe)
    {
        let ws_monitor = websocket::WebSocketMonitor::new(
            rpc_manager.clone(),
            pumpfun_client.clone(),
            metrics.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) = ws_monitor.run().await {
                error!("WebSocket monitor error: {}", e);
            }
        });
    }

    // Start token event consumer: wires the WebSocket-discovered token events
    // into the OrderManager execution path.
    // AUTO_SNIPE=true enables automatic sniper orders on new token launches.
    {
        let manager = order_manager.clone();
        let pf = pumpfun_client.clone();
        let auto_snipe = std::env::var("AUTO_SNIPE").map(|v| v == "true" || v == "1").unwrap_or(false);
        let snipe_amount = std::env::var("SNIPE_AMOUNT_SOL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .map(|sol| (sol * 1_000_000_000.0) as u64)
            .unwrap_or(50_000_000); // default 0.05 SOL
        tokio::spawn(async move {
            info!("Token event consumer started (auto_snipe={}, amount={})", auto_snipe, snipe_amount);
            manager.start_token_event_consumer(pf, auto_snipe, snipe_amount).await;
        });
    }

    // Start position update loop
    {
        let manager = order_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let Err(e) = manager.update_positions().await {
                    error!("Position update error: {}", e);
                }
            }
        });
    }

    // Start DB cleanup loop
    {
        let pool = db_pool.pool.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                if let Err(e) = database::cleanup_old_data(&pool).await {
                    error!("DB cleanup error: {}", e);
                }
            }
        });
    }

    // Build gRPC service
    let bot_service = BotService::new(
        order_manager.clone(),
        pumpfun_client.clone(),
        metrics.clone(),
    );

    let grpc_addr = format!("0.0.0.0:{}", config.grpc_port)
        .parse()
        .map_err(|e| format!("Invalid gRPC address: {}", e))?;
    info!("gRPC server listening on {}", grpc_addr);

    // Graceful shutdown via Ctrl+C
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for Ctrl+C");
        info!("Shutdown signal received");
        let _ = shutdown_tx.send(());
    });

    Server::builder()
        .add_service(BotServer::new(bot_service))
        .serve_with_shutdown(grpc_addr, async {
            shutdown_rx.await.ok();
            info!("gRPC server shutting down gracefully");
        })
        .await?;

    info!("PumpFun Trading Engine stopped");
    Ok(())
}
