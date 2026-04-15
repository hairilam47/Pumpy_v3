use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
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

/// Shared immutable configuration threaded to the supervisor and watcher tasks.
/// Does NOT own JoinSet, active_ids, or failure_counts — those live exclusively
/// inside the supervisor task to avoid Mutex-held-across-await deadlocks.
#[derive(Clone)]
struct WorkerFactory {
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
}

impl WorkerFactory {
    /// Attempt to build a `WalletWorker` from a registry entry.
    /// Returns `None` if the keypair file cannot be read or is invalid.
    /// Never logs the keypair bytes; logs the wallet_id and file path for diagnosis.
    fn build(&self, entry: WalletFullEntry) -> Option<WalletWorker> {
        let wallet_id = entry.wallet_id.clone();

        let kp_path = match &entry.keypair_path {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                info!(
                    wallet_id = %wallet_id,
                    "wallet_registry: no keypair_path — skipping worker spawn"
                );
                return None;
            }
        };

        let raw = match std::fs::read_to_string(&kp_path) {
            Ok(s) => s,
            Err(e) => {
                error!(wallet_id = %wallet_id, path = %kp_path, "Failed to read keypair file: {}", e);
                return None;
            }
        };

        let keypair_bytes: Vec<u8> = match serde_json::from_str(raw.trim()) {
            Ok(b) => b,
            Err(e) => {
                error!(wallet_id = %wallet_id, "Keypair file invalid JSON: {}", e);
                return None;
            }
        };

        if keypair_bytes.len() != 64 {
            error!(
                wallet_id = %wallet_id,
                got = keypair_bytes.len(),
                "Keypair must be 64 bytes — skipping"
            );
            return None;
        }

        Some(WalletWorker::new(
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
        ))
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
    // Serves two purposes:
    //   (1) WebSocket monitor — discovers tokens and broadcasts to all workers.
    //   (2) gRPC management plane — BotService uses its pubkey for status display.
    let primary_pumpfun_client = Arc::new(
        PumpFunClient::new(rpc_manager.clone(), config.keypair_bytes.clone())
            .map_err(|e| format!("PumpFun client error: {}", e))?,
    );
    info!("Primary PumpFun client: wallet={}", primary_pumpfun_client.pubkey());

    // ── Primary OrderManager — gRPC management plane only ─────────────────
    // Used exclusively by BotService for order submission, cancellation, and
    // portfolio queries via gRPC.  Execution workers are NOT started on this
    // manager; all trade execution happens through registry-based WalletWorkers.
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
    info!("Primary order manager initialized (gRPC management plane — no background consumers)");

    // ── WebSocket token monitor ───────────────────────────────────────────
    // Single instance; broadcasts new-token events to all WalletWorkers via
    // the shared PumpFunClient's broadcast channel.
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

    // ── DB cleanup loop ───────────────────────────────────────────────────
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

    // ── Wallet Registry Bootstrap ─────────────────────────────────────────
    // Backwards-compat: if registry is empty, auto-register the primary wallet
    // as 'wallet_001' (with KEYPAIR_PATH if set) so the orchestrator can
    // discover and spawn it uniformly as a registry worker.
    let primary_pubkey = primary_pumpfun_client.pubkey().to_string();
    {
        let all_wallets = database::load_wallet_registry(&db_pool.pool).await;
        if all_wallets.is_empty() {
            info!("wallet_registry is empty — auto-registering primary wallet as 'wallet_001'");
            let keypair_path = std::env::var("KEYPAIR_PATH").ok();
            match database::upsert_wallet_registry(
                &db_pool.pool,
                "wallet_001",
                keypair_path.as_deref(),
                Some(&primary_pubkey),
            )
            .await
            {
                Ok(_) => info!("wallet_registry: wallet_001 registered (primary env-var wallet)"),
                Err(e) => warn!("Could not auto-register wallet_001: {}", e),
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

    // ── Global auto_snipe / snipe_amount ─────────────────────────────────
    let auto_snipe = std::env::var("AUTO_SNIPE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let snipe_amount = std::env::var("SNIPE_AMOUNT_SOL")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|sol| (sol * 1_000_000_000.0) as u64)
        .unwrap_or(50_000_000);

    // ── WorkerFactory — shared immutable config for supervisor + watcher ──
    let factory = Arc::new(WorkerFactory {
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
    });

    // ── Spawn-request channel ─────────────────────────────────────────────
    // Watcher and backoff-restart tasks send WalletFullEntry values to the
    // supervisor, which is the SOLE owner of the JoinSet.  No Mutex is ever
    // held across an async await boundary.
    let (spawn_tx, spawn_rx) = mpsc::channel::<WalletFullEntry>(128);

    // ── Supervisor task ───────────────────────────────────────────────────
    // Owns the JoinSet<String>, active_ids, and failure_counts directly.
    // Receives spawn requests via the channel; detects exits via join_next().
    // Implements true exponential-backoff restarts (actual sleep, not watcher
    // polling).  Halts the wallet in DB after MAX_RESTART_ATTEMPTS.
    {
        let factory_sup = factory.clone();
        let db_pool_sup = db_pool.clone();
        let spawn_tx_restart = spawn_tx.clone();
        let mut spawn_rx = spawn_rx;

        tokio::spawn(async move {
            let mut js: JoinSet<String> = JoinSet::new();
            let mut active_ids: HashSet<String> = HashSet::new();
            let mut failure_counts: HashMap<String, u32> = HashMap::new();

            info!("Orchestrator supervisor started");

            loop {
                tokio::select! {
                    // ── Receive a spawn request ──────────────────────────
                    msg = spawn_rx.recv() => {
                        let entry = match msg {
                            Some(e) => e,
                            None => {
                                warn!("Spawn channel closed — supervisor exiting");
                                break;
                            }
                        };

                        let wallet_id = entry.wallet_id.clone();

                        // Deduplicate: skip if already running.
                        if active_ids.contains(&wallet_id) {
                            continue;
                        }

                        // Build the worker; skip if keypair file missing/invalid.
                        let worker = match factory_sup.build(entry) {
                            Some(w) => w,
                            None => continue,
                        };

                        info!(wallet_id = %wallet_id, "Supervisor: spawning WalletWorker");
                        active_ids.insert(wallet_id.clone());

                        // Spawn the worker.  We catch any panic via catch_unwind so
                        // that the wallet_id is always returned to join_next(), even
                        // on panic, enabling correct failure counting.
                        let wid = wallet_id.clone();
                        js.spawn(async move {
                            use futures::FutureExt;
                            match std::panic::AssertUnwindSafe(worker.run())
                                .catch_unwind()
                                .await
                            {
                                Ok(id) => id,
                                Err(_panic) => {
                                    error!(wallet_id = %wid, "WalletWorker panicked");
                                    wid
                                }
                            }
                        });
                    }

                    // ── Detect WalletWorker exits ────────────────────────
                    Some(result) = js.join_next() => {
                        let wallet_id = match result {
                            Ok(id) => id,
                            Err(e) => {
                                // Should not normally happen after catch_unwind,
                                // but handle it safely.
                                error!("Supervisor: unexpected JoinError: {:?}", e);
                                continue;
                            }
                        };

                        active_ids.remove(&wallet_id);
                        warn!(wallet_id = %wallet_id, "Supervisor: WalletWorker exited");

                        let count = failure_counts.entry(wallet_id.clone()).or_insert(0);
                        *count += 1;
                        let attempts = *count;

                        if attempts >= MAX_RESTART_ATTEMPTS {
                            error!(
                                wallet_id = %wallet_id,
                                attempts,
                                decision = "HALT",
                                "Supervisor: WalletWorker exceeded MAX_RESTART_ATTEMPTS — halting wallet"
                            );
                            database::halt_wallet(&db_pool_sup.pool, &wallet_id).await;
                        } else {
                            let backoff_secs = 2u64.pow(attempts);
                            warn!(
                                wallet_id = %wallet_id,
                                attempt = attempts,
                                backoff_secs,
                                "Supervisor: WalletWorker will restart after exponential backoff"
                            );
                            // Spawn a one-shot timer that will re-queue the wallet
                            // after the backoff elapses.  This is a true sleep —
                            // the watcher's 30s poll interval is NOT used for this.
                            let tx = spawn_tx_restart.clone();
                            let db = db_pool_sup.clone();
                            let wid = wallet_id.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                                // Re-fetch entry; if wallet was disabled/halted, skip.
                                let entries =
                                    database::load_enabled_wallet_full_entries(&db.pool).await;
                                if let Some(entry) =
                                    entries.into_iter().find(|e| e.wallet_id == wid)
                                {
                                    info!(
                                        wallet_id = %wid,
                                        "Supervisor: backoff elapsed — re-queuing WalletWorker"
                                    );
                                    let _ = tx.send(entry).await;
                                } else {
                                    warn!(
                                        wallet_id = %wid,
                                        "Supervisor: wallet no longer enabled after backoff — not restarting"
                                    );
                                }
                            });
                        }
                    }
                }
            }
        });
    }

    // ── Initial load: queue workers for all enabled wallets ───────────────
    {
        let entries = database::load_enabled_wallet_full_entries(&db_pool.pool).await;
        let total = entries.len();
        let spawnable = entries.iter().filter(|e| {
            e.keypair_path.as_ref().map(|p| !p.is_empty()).unwrap_or(false)
        }).count();
        info!(
            total,
            with_keypair_path = spawnable,
            "Orchestrator: queuing initial wallet workers"
        );
        for entry in entries {
            let _ = spawn_tx.send(entry).await;
        }
    }

    // ── Watcher task: polls for newly-enabled wallets every 30s ──────────
    // Sends candidates to the supervisor; supervisor deduplicates by active_ids.
    // Note: restarts with backoff are handled by the supervisor, NOT this watcher.
    {
        let db_pool_watch = db_pool.clone();
        let tx_watch = spawn_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.tick().await; // skip immediate first tick
            loop {
                interval.tick().await;
                let entries =
                    database::load_enabled_wallet_full_entries(&db_pool_watch.pool).await;
                for entry in entries {
                    let _ = tx_watch.send(entry).await;
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
