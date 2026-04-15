use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tonic::transport::Server;
use tracing::{info, error, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod config;
mod constants;
mod database;
mod decision;
mod grpc_server;
mod mev;
mod metrics;
mod order;
mod pumpfun;
mod rpc;
mod transaction;
mod wallet_worker;
mod websocket;

use config::Config;
use database::{DatabasePool, WalletFullEntry};
use decision::DecisionEngine;
use grpc_server::{BotService, bot_proto::bot_server::BotServer};
use mev::MevProtector;
use metrics::Metrics;
use order::{OrderManager, manager::OrderManagerConfig};
use pumpfun::PumpFunClient;
use rpc::RpcManager;
use wallet_worker::{WalletWorker, WalletWorkerConfig, MAX_RESTART_ATTEMPTS};

/// Shared orchestrator state threaded through the initial-load and watcher paths.
struct Orchestrator {
    db_pool: DatabasePool,
    rpc_manager: Arc<RpcManager>,
    metrics: Arc<Metrics>,
    jito_bundle_url: Option<String>,
    mev_enabled: bool,
    order_timeout: Duration,
    max_retries: u32,
    retry_delay: Duration,
    max_sandwich: u32,
    max_slippage: u64,
    max_portfolio: f64,
    execution_workers: usize,
    token_discovery_client: Arc<PumpFunClient>,
    auto_snipe: bool,
    snipe_amount_lamports: u64,
    /// Wallet IDs currently running (guarded by Mutex for watcher-task access).
    active_ids: Arc<Mutex<HashSet<String>>>,
    /// Failure counts per wallet (for exponential backoff + halt logic).
    failure_counts: Arc<Mutex<HashMap<String, u32>>>,
    /// JoinSet tracking all WalletWorker tasks.
    join_set: Arc<Mutex<JoinSet<String>>>,
}

impl Orchestrator {
    /// Try to spawn a WalletWorker for the given registry entry.
    /// Returns `true` if the worker was successfully spawned.
    async fn try_spawn_worker(&self, entry: WalletFullEntry) -> bool {
        let wallet_id = entry.wallet_id.clone();

        // Workers without a keypair file are handled by the primary (env-var) path.
        let kp_path = match &entry.keypair_path {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                info!(
                    wallet_id = %wallet_id,
                    "wallet_registry: no keypair_path — primary env-var path handles this wallet"
                );
                return false;
            }
        };

        // Load keypair bytes from disk.
        let raw = match std::fs::read_to_string(&kp_path) {
            Ok(s) => s,
            Err(e) => {
                error!(wallet_id = %wallet_id, path = %kp_path, "Failed to read keypair file: {}", e);
                return false;
            }
        };
        let keypair_bytes: Vec<u8> = match serde_json::from_str(raw.trim()) {
            Ok(b) => b,
            Err(e) => {
                error!(wallet_id = %wallet_id, "Keypair file invalid JSON: {}", e);
                return false;
            }
        };
        if keypair_bytes.len() != 64 {
            error!(
                wallet_id = %wallet_id,
                got = keypair_bytes.len(),
                "Keypair must be 64 bytes — skipping"
            );
            return false;
        }

        let worker = WalletWorker::new(
            WalletWorkerConfig {
                wallet_id: wallet_id.clone(),
                keypair_bytes,
                risk_per_trade_sol: entry.risk_per_trade_sol,
                daily_loss_limit_sol: entry.daily_loss_limit_sol,
                strategy_preset: entry.strategy_preset.clone(),
                execution_workers: self.execution_workers,
                auto_snipe: self.auto_snipe,
                snipe_amount_lamports: self.snipe_amount_lamports,
            },
            self.db_pool.clone(),
            self.rpc_manager.clone(),
            self.metrics.clone(),
            self.jito_bundle_url.clone(),
            self.mev_enabled,
            self.order_timeout,
            self.max_retries,
            self.retry_delay,
            self.max_sandwich,
            self.max_slippage,
            self.max_portfolio,
            self.token_discovery_client.clone(),
        );

        // Mark as active before spawning to prevent duplicate spawning by the watcher.
        {
            let mut ids = self.active_ids.lock().await;
            ids.insert(wallet_id.clone());
        }

        info!(wallet_id = %wallet_id, "Orchestrator: spawning WalletWorker");
        let mut js = self.join_set.lock().await;
        js.spawn(async move { worker.run().await });

        true
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::registry()
        .with(fmt::layer().json().with_current_span(true))
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    info!("Starting PumpFun Trading Engine v{}", env!("CARGO_PKG_VERSION"));

    let mut config = Config::from_env().map_err(|e| format!("Config error: {}", e))?;
    info!("Environment: {}", config.environment);
    info!("gRPC port: {}", config.grpc_port);
    info!("Metrics port: {}", config.metrics_port);
    if config.demo_mode {
        warn!("┌─────────────────────────────────────────────────────────────┐");
        warn!("│  DEMO MODE: no wallet configured — trade execution disabled │");
        warn!("│  Set WALLET_PRIVATE_KEY in Replit Secrets to enable trading │");
        warn!("└─────────────────────────────────────────────────────────────┘");
    }

    // ── Prometheus metrics ────────────────────────────────────────────────
    let metrics = Arc::new(Metrics::new().map_err(|e| format!("Metrics error: {}", e))?);
    {
        let m = metrics.clone();
        let port = config.metrics_port;
        tokio::spawn(async move { m.start_server(port).await; });
    }

    // ── Database + config overrides ───────────────────────────────────────
    let db_pool = match DatabasePool::new(&config.database_url).await {
        Ok(pool) => {
            if let Err(e) = database::run_migrations(&pool).await {
                error!("Migration warning: {}", e);
            }
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
            DatabasePool::new("postgresql://localhost:5432/pumpfun?connect_timeout=1")
                .await
                .unwrap_or_else(|_| panic!("Cannot initialize database connection pool"))
        }
    };

    // ── RPC manager ───────────────────────────────────────────────────────
    let rpc_manager = Arc::new(
        RpcManager::new(config.rpc_endpoints.clone())
            .await
            .map_err(|e| format!("RPC manager error: {}", e))?,
    );
    let rpc_manager = Arc::new(rpc_manager.as_ref().clone().with_metrics(metrics.clone()));
    rpc_manager.start_health_checks();
    info!("RPC manager initialized with {} endpoints", config.rpc_endpoints.len());

    // ── Primary PumpFun client ────────────────────────────────────────────
    // (1) WebSocket monitor — broadcasts token-discovery events to all workers.
    // (2) Primary (env-var/ephemeral) wallet — used by gRPC management plane.
    let primary_pumpfun_client = Arc::new(
        PumpFunClient::new(rpc_manager.clone(), config.keypair_bytes.clone())
            .map_err(|e| format!("PumpFun client error: {}", e))?,
    );
    info!("Primary PumpFun client: wallet={}", primary_pumpfun_client.pubkey());

    // ── Primary MEV + Decision Engine + OrderManager ──────────────────────
    let jito_client_opt: Option<Arc<crate::mev::JitoClient>> = config
        .jito_bundle_url
        .as_ref()
        .map(|url| Arc::new(crate::mev::JitoClient::new(url.clone())));

    let mev_protector = Arc::new(MevProtector::new(
        config.jito_bundle_url.clone(),
        primary_pumpfun_client.clone(),
        metrics.clone(),
        config.risk_limits.max_sandwich_risk_score,
        config.trading.mev_protection_enabled,
    ));
    info!("MEV protector initialized (Jito: {})", mev_protector.has_jito());

    let decision_engine = Arc::new(DecisionEngine::new());
    info!("Decision Engine initialized");

    let order_config = OrderManagerConfig {
        max_pending_orders: 100,
        order_timeout: Duration::from_secs(config.order_timeout_seconds),
        max_retries: config.trading.retry_attempts,
        retry_delay: Duration::from_millis(config.trading.retry_delay_ms),
        max_position_size_sol: config.risk_limits.max_position_size_sol,
        max_portfolio_exposure_sol: config.risk_limits.max_portfolio_exposure_sol,
        max_daily_loss_sol: config.risk_limits.max_daily_loss_sol,
        max_sandwich_risk_score: config.risk_limits.max_sandwich_risk_score,
        max_slippage_bps: config.risk_limits.max_slippage_bps,
    };

    let order_manager = Arc::new(OrderManager::new(
        db_pool.clone(),
        primary_pumpfun_client.clone(),
        mev_protector.clone(),
        jito_client_opt.clone(),
        metrics.clone(),
        order_config.clone(),
        config.trading.mev_protection_enabled,
        decision_engine,
        config.demo_mode,
    ));
    info!("Primary order manager initialized");

    // ── Primary wallet sub-tasks (env-var/ephemeral path) ─────────────────
    let semaphore = Arc::new(tokio::sync::Semaphore::new(config.execution_workers));
    for i in 0..config.execution_workers {
        let manager = order_manager.clone();
        let sem = semaphore.clone();
        tokio::spawn(async move {
            info!("Primary execution worker {} started", i);
            manager.execution_worker(sem).await;
        });
    }

    // WebSocket monitor — single instance, broadcasts to all wallets.
    {
        let ws_monitor = websocket::WebSocketMonitor::new(
            rpc_manager.clone(),
            primary_pumpfun_client.clone(),
            metrics.clone(),
        );
        tokio::spawn(async move {
            if let Err(e) = ws_monitor.run().await {
                error!("WebSocket monitor error: {}", e);
            }
        });
    }

    // Primary token event consumer.
    {
        let manager = order_manager.clone();
        let pf = primary_pumpfun_client.clone();
        let auto_snipe = std::env::var("AUTO_SNIPE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let snipe_amount = std::env::var("SNIPE_AMOUNT_SOL")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .map(|sol| (sol * 1_000_000_000.0) as u64)
            .unwrap_or(50_000_000);
        tokio::spawn(async move {
            info!("Primary token event consumer started (auto_snipe={})", auto_snipe);
            manager.start_token_event_consumer(pf, auto_snipe, snipe_amount).await;
        });
    }

    // Primary position update loop.
    {
        let manager = order_manager.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let Err(e) = manager.update_positions().await {
                    error!("Primary position update error: {}", e);
                }
            }
        });
    }

    // DB cleanup loop.
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

    // ── Wallet Registry Orchestration ─────────────────────────────────────
    // Backwards-compat: if registry is empty, auto-register the primary
    // wallet as 'wallet_001' so operators can see and extend it.
    let primary_pubkey = primary_pumpfun_client.pubkey().to_string();
    {
        let all_wallets = database::load_wallet_registry(&db_pool.pool).await;
        if all_wallets.is_empty() {
            info!("wallet_registry is empty — auto-registering primary wallet as 'wallet_001'");
            let keypair_path = std::env::var("KEYPAIR_PATH").ok();
            if let Err(e) = database::upsert_wallet_registry(
                &db_pool.pool,
                "wallet_001",
                keypair_path.as_deref(),
                Some(&primary_pubkey),
            )
            .await
            {
                warn!("Could not auto-register wallet_001: {}", e);
            } else {
                info!("wallet_registry: wallet_001 registered (primary env-var wallet)");
            }
        } else {
            info!("wallet_registry: {} registered wallet(s)", all_wallets.len());
            for w in &all_wallets {
                info!(
                    wallet_id = %w.wallet_id,
                    status = %w.status,
                    owner_pubkey = ?w.owner_pubkey,
                    "wallet_registry entry"
                );
            }
        }
    }

    // Global auto_snipe / snipe_amount (shared with WalletWorkers).
    let auto_snipe = std::env::var("AUTO_SNIPE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let snipe_amount = std::env::var("SNIPE_AMOUNT_SOL")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|sol| (sol * 1_000_000_000.0) as u64)
        .unwrap_or(50_000_000);

    // Build the orchestrator (holds shared mutable state for workers).
    let orchestrator = Arc::new(Orchestrator {
        db_pool: db_pool.clone(),
        rpc_manager: rpc_manager.clone(),
        metrics: metrics.clone(),
        jito_bundle_url: config.jito_bundle_url.clone(),
        mev_enabled: config.trading.mev_protection_enabled,
        order_timeout: Duration::from_secs(config.order_timeout_seconds),
        max_retries: config.trading.retry_attempts,
        retry_delay: Duration::from_millis(config.trading.retry_delay_ms),
        max_sandwich: config.risk_limits.max_sandwich_risk_score,
        max_slippage: config.risk_limits.max_slippage_bps,
        max_portfolio: config.risk_limits.max_portfolio_exposure_sol,
        execution_workers: config.execution_workers,
        token_discovery_client: primary_pumpfun_client.clone(),
        auto_snipe,
        snipe_amount_lamports: snipe_amount,
        active_ids: Arc::new(Mutex::new(HashSet::new())),
        failure_counts: Arc::new(Mutex::new(HashMap::new())),
        join_set: Arc::new(Mutex::new(JoinSet::new())),
    });

    // Initial load: spawn workers for all enabled registry wallets with keypair_path.
    {
        let entries = database::load_enabled_wallet_full_entries(&db_pool.pool).await;
        let spawnable = entries.iter().filter(|e| {
            e.keypair_path.as_ref().map(|p| !p.is_empty()).unwrap_or(false)
        }).count();
        info!("Orchestrator: {} enabled wallet(s) with keypair_path in registry", spawnable);
        for entry in entries {
            orchestrator.try_spawn_worker(entry).await;
        }
    }

    // JoinSet monitor: detects unexpected WalletWorker exits.
    {
        let orch = orchestrator.clone();
        let db_pool_mon = db_pool.clone();
        tokio::spawn(async move {
            loop {
                let result = {
                    let mut js = orch.join_set.lock().await;
                    js.join_next().await
                };

                match result {
                    Some(Ok(wallet_id)) => {
                        warn!(wallet_id = %wallet_id, "WalletWorker exited unexpectedly");
                        {
                            let mut ids = orch.active_ids.lock().await;
                            ids.remove(&wallet_id);
                        }

                        let attempts = {
                            let mut counts = orch.failure_counts.lock().await;
                            let c = counts.entry(wallet_id.clone()).or_insert(0);
                            *c += 1;
                            *c
                        };

                        if attempts >= MAX_RESTART_ATTEMPTS {
                            error!(
                                wallet_id = %wallet_id,
                                attempts,
                                decision = "HALT",
                                "WalletWorker exceeded max restart attempts — halting wallet"
                            );
                            database::halt_wallet(&db_pool_mon.pool, &wallet_id).await;
                        } else {
                            let backoff = Duration::from_secs(2u64.pow(attempts));
                            warn!(
                                wallet_id = %wallet_id,
                                attempt = attempts,
                                backoff_secs = backoff.as_secs(),
                                "WalletWorker will be retried after backoff (watcher will restart)"
                            );
                            // The watcher task will attempt to re-spawn after the backoff elapses
                            // and the wallet is no longer in active_ids.
                        }
                    }
                    Some(Err(e)) if e.is_panic() => {
                        error!("WalletWorker panicked: {:?}", e);
                    }
                    Some(Err(e)) => {
                        warn!("WalletWorker task join error: {:?}", e);
                    }
                    None => {
                        // JoinSet is empty — park briefly.
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }

    // Watcher task: polls wallet_registry every 30s for newly-enabled wallets.
    // Supports adding wallets at runtime without restarting the engine.
    {
        let orch = orchestrator.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                let entries = database::load_enabled_wallet_full_entries(&orch.db_pool.pool).await;
                for entry in entries {
                    let wallet_id = entry.wallet_id.clone();
                    let is_active = {
                        let ids = orch.active_ids.lock().await;
                        ids.contains(&wallet_id)
                    };
                    let is_permanently_halted = {
                        let counts = orch.failure_counts.lock().await;
                        counts.get(&wallet_id).copied().unwrap_or(0) >= MAX_RESTART_ATTEMPTS
                    };
                    if !is_active && !is_permanently_halted {
                        info!(
                            wallet_id = %wallet_id,
                            "Orchestrator watcher: new wallet detected — spawning worker"
                        );
                        orch.try_spawn_worker(entry).await;
                    }
                }
            }
        });
    }

    // ── gRPC server (blocks until shutdown) ───────────────────────────────
    let bot_service = BotService::new(
        order_manager.clone(),
        primary_pumpfun_client.clone(),
        metrics.clone(),
        config.demo_mode,
    );

    let grpc_addr = format!("0.0.0.0:{}", config.grpc_port)
        .parse()
        .map_err(|e| format!("Invalid gRPC address: {}", e))?;
    info!("gRPC server listening on {}", grpc_addr);

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
