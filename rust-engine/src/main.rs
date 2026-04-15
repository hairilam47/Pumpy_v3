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

enum BuildOutcome {
    Built(WalletWorker),
    Failed(String),
}

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
    auto_pause_threshold: u32,
}

impl WorkerFactory {
    fn build(&self, entry: WalletFullEntry) -> BuildOutcome {
        let wallet_id = entry.wallet_id.clone();

        let kp_path = match &entry.keypair_path {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                return BuildOutcome::Failed(format!("wallet '{}' has no keypair_path", wallet_id));
            }
        };

        let raw = match std::fs::read_to_string(&kp_path) {
            Ok(s) => s,
            Err(e) => {
                error!(wallet_id = %wallet_id, "Failed to read keypair file: {}", e);
                return BuildOutcome::Failed(format!("read error: {}", e));
            }
        };

        let keypair_bytes: Vec<u8> = match serde_json::from_str(raw.trim()) {
            Ok(b) => b,
            Err(e) => {
                error!(wallet_id = %wallet_id, "Keypair file invalid JSON: {}", e);
                return BuildOutcome::Failed(format!("invalid JSON: {}", e));
            }
        };

        if keypair_bytes.len() != 64 {
            error!(wallet_id = %wallet_id, got = keypair_bytes.len(), "Keypair must be 64 bytes");
            return BuildOutcome::Failed(format!("expected 64 bytes, got {}", keypair_bytes.len()));
        }

        BuildOutcome::Built(WalletWorker::new(
            WalletWorkerConfig {
                wallet_id: wallet_id.clone(),
                keypair_bytes,
                risk_per_trade_sol: entry.risk_per_trade_sol,
                daily_loss_limit_sol: entry.daily_loss_limit_sol,
                strategy_preset: entry.strategy_preset.clone(),
                execution_workers: self.execution_workers,
                auto_snipe: self.auto_snipe,
                snipe_amount_lamports: self.snipe_amount_lamports,
                auto_pause_threshold: self.auto_pause_threshold,
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

/// Write keypair bytes to a secure temp file.
/// Uses a nanosecond-nonce suffix, mode 0o600.  Path is not logged.
/// Caller is responsible for cleanup (see cleanup_temp_keypair).
fn materialize_keypair(keypair_bytes: &[u8]) -> Option<String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let path = format!("/tmp/.kp{:08x}", nonce);
    let json = serde_json::to_string(&keypair_bytes.to_vec()).ok()?;

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .ok()?;
    f.write_all(json.as_bytes()).ok()?;
    Some(path)
}

fn cleanup_temp_keypair(path: Option<&str>) {
    if let Some(p) = path {
        if p.starts_with("/tmp/.kp") {
            let _ = std::fs::remove_file(p);
        }
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
        warn!("DEMO MODE — ephemeral keypair, trading disabled; set WALLET_PRIVATE_KEY to enable");
    }

    let metrics = Arc::new(Metrics::new().map_err(|e| format!("Metrics error: {}", e))?);
    {
        let m = metrics.clone();
        let port = config.metrics_port;
        tokio::spawn(async move { m.start_server(port).await; });
    }

    let mut auto_pause_threshold: u32 = 10;

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
            if let Some(v) = db_overrides.get("auto_pause_threshold").or_else(|| db_overrides.get("AUTO_PAUSE_THRESHOLD")) {
                if let Ok(n) = v.parse::<u32>() {
                    auto_pause_threshold = n;
                    info!("auto_pause_threshold set to {} from system_config", n);
                }
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

    let rpc_manager = Arc::new(
        RpcManager::new(config.rpc_endpoints.clone())
            .await
            .map_err(|e| format!("RPC manager error: {}", e))?,
    );
    let rpc_manager = Arc::new(rpc_manager.as_ref().clone().with_metrics(metrics.clone()));
    rpc_manager.start_health_checks();
    info!("RPC manager initialized with {} endpoints", config.rpc_endpoints.len());

    let primary_pumpfun_client = Arc::new(
        PumpFunClient::new(rpc_manager.clone(), config.keypair_bytes.clone())
            .map_err(|e| format!("PumpFun client error: {}", e))?,
    );
    info!("Primary PumpFun client: wallet={}", primary_pumpfun_client.pubkey());

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

    // gRPC control-plane shim: consumes primary OrderManager queue for BotService::submit_order.
    {
        let sem = Arc::new(tokio::sync::Semaphore::new(config.execution_workers));
        for i in 0..config.execution_workers {
            let manager = order_manager.clone();
            let sem = sem.clone();
            tokio::spawn(async move {
                info!("Primary execution worker {} started (gRPC shim)", i);
                manager.execution_worker(sem).await;
            });
        }
    }
    info!("Primary order manager initialized");

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

    let auto_snipe = std::env::var("AUTO_SNIPE")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let snipe_amount = std::env::var("SNIPE_AMOUNT_SOL")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|sol| (sol * 1_000_000_000.0) as u64)
        .unwrap_or(50_000_000);

    // Bootstrap: if registry is empty, auto-register wallet_001 with a usable keypair_path.
    // Resolution: KEYPAIR_PATH env var → materialize bytes to a secure temp file.
    // The materialized_kp_path is kept to allow cleanup on shutdown.
    let primary_pubkey = primary_pumpfun_client.pubkey().to_string();
    let materialized_kp_path: Option<String> = {
        let all_wallets = database::load_wallet_registry(&db_pool.pool).await;
        if all_wallets.is_empty() {
            let env_path = std::env::var("KEYPAIR_PATH").ok();
            let mat_path = if env_path.is_none() {
                materialize_keypair(&config.keypair_bytes)
            } else {
                None
            };
            let keypair_path = env_path.or_else(|| mat_path.clone());

            if keypair_path.is_some() {
                info!("wallet_registry empty — registering wallet_001 with keypair");
            } else {
                warn!("wallet_registry empty — keypair not available; gRPC shim handles orders");
            }

            match database::upsert_wallet_registry(
                &db_pool.pool,
                "wallet_001",
                keypair_path.as_deref(),
                Some(&primary_pubkey),
            )
            .await
            {
                Ok(_) => info!("wallet_001 registered"),
                Err(e) => warn!("Could not auto-register wallet_001: {}", e),
            }
            mat_path
        } else {
            info!("wallet_registry: {} wallet(s)", all_wallets.len());
            None
        }
    };

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
        auto_pause_threshold,
    });

    // Two spawn channels to enforce backoff policy:
    //   spawn_tx  — watcher + initial load; entries dropped if wallet is in in_backoff.
    //   restart_tx — backoff timer only; always processed, clears in_backoff for that wallet.
    let (spawn_tx, mut spawn_rx) = mpsc::channel::<WalletFullEntry>(128);
    let (restart_tx, mut restart_rx) = mpsc::channel::<WalletFullEntry>(128);

    {
        let factory_sup = factory.clone();
        let db_pool_sup = db_pool.clone();

        tokio::spawn(async move {
            let mut js: JoinSet<String> = JoinSet::new();
            let mut active_ids: HashSet<String> = HashSet::new();
            let mut in_backoff: HashSet<String> = HashSet::new();
            let mut runtime_fails: HashMap<String, u32> = HashMap::new();
            let mut build_fails: HashMap<String, u32> = HashMap::new();
            let mut permanently_skipped: HashSet<String> = HashSet::new();

            info!("Orchestrator supervisor started");

            loop {
                tokio::select! {
                    // Timer-triggered restart: always process; clears in_backoff.
                    msg = restart_rx.recv() => {
                        let entry = match msg {
                            Some(e) => e,
                            None => { warn!("Restart channel closed"); break; }
                        };
                        let wallet_id = entry.wallet_id.clone();
                        in_backoff.remove(&wallet_id);

                        if active_ids.contains(&wallet_id) || permanently_skipped.contains(&wallet_id) {
                            continue;
                        }

                        spawn_entry(
                            entry,
                            &factory_sup,
                            &db_pool_sup,
                            &mut js,
                            &mut active_ids,
                            &mut build_fails,
                            &mut permanently_skipped,
                        ).await;
                    }

                    // Watcher / initial load: skip if backoff pending.
                    msg = spawn_rx.recv() => {
                        let entry = match msg {
                            Some(e) => e,
                            None => { warn!("Spawn channel closed"); break; }
                        };
                        let wallet_id = entry.wallet_id.clone();

                        if active_ids.contains(&wallet_id)
                            || in_backoff.contains(&wallet_id)
                            || permanently_skipped.contains(&wallet_id)
                        {
                            continue;
                        }

                        spawn_entry(
                            entry,
                            &factory_sup,
                            &db_pool_sup,
                            &mut js,
                            &mut active_ids,
                            &mut build_fails,
                            &mut permanently_skipped,
                        ).await;
                    }

                    Some(result) = js.join_next() => {
                        let wallet_id = match result {
                            Ok(id) => id,
                            Err(e) => { error!("JoinError: {:?}", e); continue; }
                        };

                        active_ids.remove(&wallet_id);
                        warn!(wallet_id = %wallet_id, "Supervisor: WalletWorker exited");

                        let count = runtime_fails.entry(wallet_id.clone()).or_insert(0);
                        *count += 1;
                        let attempts = *count;

                        if attempts >= MAX_RESTART_ATTEMPTS {
                            error!(
                                wallet_id = %wallet_id,
                                attempts,
                                decision = "HALT",
                                "Supervisor: WalletWorker exceeded restart limit"
                            );
                            permanently_skipped.insert(wallet_id.clone());
                            database::halt_wallet(&db_pool_sup.pool, &wallet_id).await;
                        } else {
                            let backoff_secs = 2u64.pow(attempts);
                            warn!(wallet_id = %wallet_id, attempt = attempts, backoff_secs, "Supervisor: restarting after backoff");
                            in_backoff.insert(wallet_id.clone());
                            let tx = restart_tx.clone();
                            let db = db_pool_sup.clone();
                            let wid = wallet_id.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                                let entries = database::load_enabled_wallet_full_entries(&db.pool).await;
                                if let Some(entry) = entries.into_iter().find(|e| e.wallet_id == wid) {
                                    let _ = tx.send(entry).await;
                                } else {
                                    warn!(wallet_id = %wid, "Supervisor: wallet no longer enabled after backoff");
                                }
                            });
                        }
                    }
                }
            }
        });
    }

    {
        let entries = database::load_enabled_wallet_full_entries(&db_pool.pool).await;
        let total = entries.len();
        let spawnable = entries.iter()
            .filter(|e| e.keypair_path.as_ref().map(|p| !p.is_empty()).unwrap_or(false))
            .count();
        info!(total, with_keypair_path = spawnable, "Orchestrator: queuing initial wallet workers");
        for entry in entries {
            let _ = spawn_tx.send(entry).await;
        }
    }

    {
        let db_pool_watch = db_pool.clone();
        let tx_watch = spawn_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.tick().await;
            loop {
                interval.tick().await;
                let entries = database::load_enabled_wallet_full_entries(&db_pool_watch.pool).await;
                for entry in entries {
                    let _ = tx_watch.send(entry).await;
                }
            }
        });
    }

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
        tokio::signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
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

    cleanup_temp_keypair(materialized_kp_path.as_deref());
    info!("PumpFun Trading Engine stopped");
    Ok(())
}

async fn spawn_entry(
    entry: WalletFullEntry,
    factory: &WorkerFactory,
    db_pool: &DatabasePool,
    js: &mut JoinSet<String>,
    active_ids: &mut HashSet<String>,
    build_fails: &mut HashMap<String, u32>,
    permanently_skipped: &mut HashSet<String>,
) {
    let wallet_id = entry.wallet_id.clone();

    match factory.build(entry) {
        BuildOutcome::Built(worker) => {
            build_fails.remove(&wallet_id);
            info!(wallet_id = %wallet_id, "Supervisor: spawning WalletWorker");
            active_ids.insert(wallet_id.clone());

            let wid = wallet_id.clone();
            js.spawn(async move {
                use futures::FutureExt;
                match std::panic::AssertUnwindSafe(worker.run()).catch_unwind().await {
                    Ok(id) => id,
                    Err(_) => { error!(wallet_id = %wid, "WalletWorker panicked"); wid }
                }
            });
        }

        BuildOutcome::Failed(reason) => {
            let count = build_fails.entry(wallet_id.clone()).or_insert(0);
            *count += 1;
            let attempts = *count;

            if attempts >= MAX_RESTART_ATTEMPTS {
                error!(wallet_id = %wallet_id, attempts, reason = %reason, decision = "HALT", "Supervisor: unrecoverable build failure");
                permanently_skipped.insert(wallet_id.clone());
                database::halt_wallet(&db_pool.pool, &wallet_id).await;
            } else {
                warn!(wallet_id = %wallet_id, attempt = attempts, max = MAX_RESTART_ATTEMPTS, reason = %reason, "Supervisor: build failed");
            }
        }
    }
}
