/// Per-wallet execution worker.
///
/// Each enabled wallet from the `wallet_registry` gets one `WalletWorker`
/// spawned as an isolated tokio task. Workers share:
///   - RpcManager (read-only routing)
///   - Metrics (counters are labelled per wallet in a future pass)
///   - The token-discovery PumpFunClient (for subscribing to new-token events)
///
/// Workers do NOT share:
///   - PumpFunClient used for trade execution (each has its own keypair)
///   - OrderManager / DecisionEngine / MevProtector (fully isolated state)
///   - Risk limits (loaded per-wallet from wallet_config)
///
/// A worker that exits unexpectedly is retried by the orchestrator (main.rs)
/// up to MAX_RESTART_ATTEMPTS times with exponential backoff; after that the
/// wallet is marked halted in the database.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{info, warn, error};

use crate::database::{self, DatabasePool};
use crate::decision::DecisionEngine;
use crate::mev::{JitoClient, MevProtector};
use crate::metrics::Metrics;
use crate::order::{OrderManager, manager::OrderManagerConfig};
use crate::pumpfun::PumpFunClient;
use crate::rpc::RpcManager;

/// Maximum number of times the orchestrator will restart a crashed worker
/// before permanently halting the wallet.
pub const MAX_RESTART_ATTEMPTS: u32 = 3;

/// Per-wallet configuration resolved from wallet_config (or defaults).
#[derive(Debug, Clone)]
pub struct WalletWorkerConfig {
    /// Wallet identifier (primary key in wallet_registry / wallet_config).
    pub wallet_id: String,
    /// Raw 64-byte keypair used for trade execution. NEVER logged.
    pub keypair_bytes: Vec<u8>,
    /// Per-wallet risk limit: maximum SOL per single trade.
    pub risk_per_trade_sol: f64,
    /// Per-wallet risk limit: maximum daily loss in SOL.
    pub daily_loss_limit_sol: f64,
    /// Human-readable strategy preset ('conservative'|'balanced'|'aggressive').
    pub strategy_preset: String,
    /// Number of concurrent execution workers (from global config).
    pub execution_workers: usize,
    /// Whether to automatically snipe new token launches.
    pub auto_snipe: bool,
    /// Snipe order size in lamports.
    pub snipe_amount_lamports: u64,
}

/// A self-contained worker that manages one wallet's full execution lifecycle.
pub struct WalletWorker {
    pub config: WalletWorkerConfig,
    db_pool: DatabasePool,
    rpc_manager: Arc<RpcManager>,
    metrics: Arc<Metrics>,
    jito_bundle_url: Option<String>,
    mev_protection_enabled: bool,
    order_timeout: Duration,
    max_retries: u32,
    retry_delay: Duration,
    max_sandwich_risk_score: u32,
    max_slippage_bps: u64,
    max_portfolio_exposure_sol: f64,
    /// Shared PumpFunClient used only for subscribing to new-token-discovery events.
    /// Trade execution uses a separate per-wallet client built inside `run()`.
    token_discovery_client: Arc<PumpFunClient>,
}

impl WalletWorker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: WalletWorkerConfig,
        db_pool: DatabasePool,
        rpc_manager: Arc<RpcManager>,
        metrics: Arc<Metrics>,
        jito_bundle_url: Option<String>,
        mev_protection_enabled: bool,
        order_timeout: Duration,
        max_retries: u32,
        retry_delay: Duration,
        max_sandwich_risk_score: u32,
        max_slippage_bps: u64,
        max_portfolio_exposure_sol: f64,
        token_discovery_client: Arc<PumpFunClient>,
    ) -> Self {
        Self {
            config,
            db_pool,
            rpc_manager,
            metrics,
            jito_bundle_url,
            mev_protection_enabled,
            order_timeout,
            max_retries,
            retry_delay,
            max_sandwich_risk_score,
            max_slippage_bps,
            max_portfolio_exposure_sol,
            token_discovery_client,
        }
    }

    /// Run the wallet worker.
    ///
    /// Builds all per-wallet components (PumpFunClient, DecisionEngine,
    /// MevProtector, OrderManager), spawns sub-tasks, and then parks the
    /// future until cancelled.  Returns the wallet_id when it exits so the
    /// orchestrator can track failures.
    pub async fn run(self) -> String {
        let wallet_id = self.config.wallet_id.clone();

        info!(
            wallet_id = %wallet_id,
            strategy = %self.config.strategy_preset,
            risk_sol = self.config.risk_per_trade_sol,
            "WalletWorker starting"
        );

        // Build per-wallet PumpFunClient for trade execution.
        let execution_client = match PumpFunClient::new(
            self.rpc_manager.clone(),
            self.config.keypair_bytes.clone(),
        ) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                error!(
                    wallet_id = %wallet_id,
                    error = %e,
                    decision = "HALT",
                    "WalletWorker: failed to build execution PumpFunClient"
                );
                database::halt_wallet(&self.db_pool.pool, &wallet_id).await;
                return wallet_id;
            }
        };

        info!(
            wallet_id = %wallet_id,
            pubkey = %execution_client.pubkey(),
            "WalletWorker: execution client ready"
        );

        // Per-wallet JitoClient (uses the same global URL if configured).
        let jito_client_opt: Option<Arc<JitoClient>> = self
            .jito_bundle_url
            .as_ref()
            .map(|url| Arc::new(JitoClient::new(url.clone())));

        // Per-wallet MevProtector.
        let mev_protector = Arc::new(MevProtector::new(
            self.jito_bundle_url.clone(),
            execution_client.clone(),
            self.metrics.clone(),
            self.max_sandwich_risk_score,
            self.mev_protection_enabled,
        ));

        // Per-wallet DecisionEngine — stateless, but isolated so future
        // per-wallet rule extensions don't bleed across wallets.
        let decision_engine = Arc::new(DecisionEngine::new());

        // Per-wallet OrderManagerConfig with risk limits from wallet_config.
        let order_config = OrderManagerConfig {
            max_pending_orders: 100,
            order_timeout: self.order_timeout,
            max_retries: self.max_retries,
            retry_delay: self.retry_delay,
            // Per-wallet limits:
            max_position_size_sol: self.config.risk_per_trade_sol,
            max_daily_loss_sol: self.config.daily_loss_limit_sol,
            // Global limits:
            max_portfolio_exposure_sol: self.max_portfolio_exposure_sol,
            max_sandwich_risk_score: self.max_sandwich_risk_score,
            max_slippage_bps: self.max_slippage_bps,
        };

        let order_manager = Arc::new(OrderManager::new(
            self.db_pool.clone(),
            execution_client.clone(),
            mev_protector.clone(),
            jito_client_opt,
            self.metrics.clone(),
            order_config,
            self.mev_protection_enabled,
            decision_engine,
            false, // keypair was loaded — not demo mode
        ));

        // Spawn execution workers (process orders from the queue).
        let semaphore = Arc::new(Semaphore::new(self.config.execution_workers));
        for i in 0..self.config.execution_workers {
            let manager = order_manager.clone();
            let sem = semaphore.clone();
            let wid = wallet_id.clone();
            tokio::spawn(async move {
                info!(wallet_id = %wid, worker = i, "Execution worker started");
                manager.execution_worker(sem).await;
            });
        }

        // Spawn token event consumer.
        // Subscribes to the shared token-discovery broadcast so all wallets
        // receive every new-token event discovered by the WebSocket monitor.
        {
            let manager = order_manager.clone();
            let discovery_client = self.token_discovery_client.clone();
            let auto_snipe = self.config.auto_snipe;
            let snipe_amount = self.config.snipe_amount_lamports;
            let wid = wallet_id.clone();
            tokio::spawn(async move {
                info!(wallet_id = %wid, auto_snipe, "Token event consumer started");
                manager
                    .start_token_event_consumer(discovery_client, auto_snipe, snipe_amount)
                    .await;
                warn!(wallet_id = %wid, "Token event consumer exited");
            });
        }

        // Spawn position update loop.
        {
            let manager = order_manager.clone();
            let wid = wallet_id.clone();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(5));
                loop {
                    interval.tick().await;
                    if let Err(e) = manager.update_positions().await {
                        error!(wallet_id = %wid, "Position update error: {}", e);
                    }
                }
            });
        }

        info!(
            wallet_id = %wallet_id,
            "WalletWorker: all sub-tasks spawned, worker is live"
        );

        // Park until cancelled by the orchestrator (e.g., on shutdown).
        std::future::pending::<()>().await;

        // Unreachable in normal operation — only reached if the future is
        // explicitly cancelled, at which point the wallet_id is returned so
        // the orchestrator can log the exit.
        wallet_id
    }
}
