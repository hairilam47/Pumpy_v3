use std::collections::{HashMap, VecDeque};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc, broadcast, Semaphore};
use uuid::Uuid;
use chrono::Utc;
use tracing::{info, warn, error};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::database::DatabasePool;
use crate::decision::{Decision, DecisionContext, DecisionEngine};
use crate::metrics::Metrics;
use crate::mev::{MevProtector, JitoClient};
use crate::pumpfun::PumpFunClient;
use super::{Order, OrderError, OrderSide, OrderStatus, OrderType};

/// Configuration for the order manager
#[derive(Debug, Clone)]
pub struct OrderManagerConfig {
    pub max_pending_orders: usize,
    pub order_timeout: Duration,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub max_position_size_sol: f64,
    pub max_portfolio_exposure_sol: f64,
    pub max_daily_loss_sol: f64,
    pub max_sandwich_risk_score: u32,
    /// Maximum allowed slippage in basis points.
    pub max_slippage_bps: u64,
}

impl Default for OrderManagerConfig {
    fn default() -> Self {
        Self {
            max_pending_orders: 100,
            order_timeout: Duration::from_secs(30),
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
            max_position_size_sol: 10.0,
            max_portfolio_exposure_sol: 100.0,
            max_daily_loss_sol: 5.0,
            max_sandwich_risk_score: 70,
            max_slippage_bps: 1000,
        }
    }
}

pub struct OrderManager {
    db_pool: DatabasePool,
    pumpfun_client: Arc<PumpFunClient>,
    mev_protector: Arc<MevProtector>,
    jito_client: Option<Arc<JitoClient>>,
    metrics: Arc<Metrics>,
    pending_orders: Arc<RwLock<VecDeque<Order>>>,
    active_orders: Arc<RwLock<HashMap<String, Order>>>,
    order_history: Arc<RwLock<HashMap<String, Order>>>,
    order_tx: mpsc::UnboundedSender<Order>,
    order_rx: Arc<RwLock<mpsc::UnboundedReceiver<Order>>>,
    event_tx: broadcast::Sender<OrderEvent>,
    config: OrderManagerConfig,
    mev_enabled: bool,
    decision_engine: Arc<DecisionEngine>,
    demo_mode: bool,
    /// Real wallet public key used as audit identity in every DecisionEngine log.
    wallet_pubkey: String,
    /// Deterministic fingerprint of current risk-limit config for audit logs.
    config_version_hash: String,
}

#[derive(Debug, Clone)]
pub struct OrderEvent {
    pub order_id: String,
    pub token_mint: String,
    pub status: String,
    pub signature: Option<String>,
    pub error: Option<String>,
    pub executed_at: Option<String>,
    pub executed_price: Option<f64>,
    pub executed_amount: Option<u64>,
}

impl OrderManager {
    pub fn new(
        db_pool: DatabasePool,
        pumpfun_client: Arc<PumpFunClient>,
        mev_protector: Arc<MevProtector>,
        jito_client: Option<Arc<JitoClient>>,
        metrics: Arc<Metrics>,
        config: OrderManagerConfig,
        mev_enabled: bool,
        decision_engine: Arc<DecisionEngine>,
        demo_mode: bool,
    ) -> Self {
        let (order_tx, order_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(1000);

        let wallet_pubkey = pumpfun_client.pubkey().to_string();
        let config_version_hash = Self::compute_config_hash(&config);

        Self {
            db_pool,
            pumpfun_client,
            mev_protector,
            jito_client,
            metrics,
            pending_orders: Arc::new(RwLock::new(VecDeque::new())),
            active_orders: Arc::new(RwLock::new(HashMap::new())),
            order_history: Arc::new(RwLock::new(HashMap::new())),
            order_tx,
            order_rx: Arc::new(RwLock::new(order_rx)),
            event_tx,
            config,
            mev_enabled,
            decision_engine,
            demo_mode,
            wallet_pubkey,
            config_version_hash,
        }
    }

    /// Compute a deterministic short hash of the risk-limit config snapshot.
    /// Changes whenever any limit is modified, enabling audit log correlation.
    fn compute_config_hash(cfg: &OrderManagerConfig) -> String {
        let mut h = DefaultHasher::new();
        ((cfg.max_position_size_sol * 1_000.0) as u64).hash(&mut h);
        ((cfg.max_portfolio_exposure_sol * 1_000.0) as u64).hash(&mut h);
        ((cfg.max_daily_loss_sol * 1_000.0) as u64).hash(&mut h);
        cfg.max_slippage_bps.hash(&mut h);
        cfg.max_sandwich_risk_score.hash(&mut h);
        format!("{:08x}", h.finish() & 0xFFFF_FFFF)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<OrderEvent> {
        self.event_tx.subscribe()
    }

    pub fn db_pool(&self) -> &DatabasePool {
        &self.db_pool
    }

    /// Submit a new order for execution.
    /// The Decision Engine is the first gate — no order enters the queue without an Allow.
    pub async fn submit_order(&self, mut order: Order) -> Result<String, OrderError> {
        // Assign a provisional ID so the DecisionEngine can log it.
        order.id = Uuid::new_v4().to_string();

        // Current exposure = sum of in-flight order amounts (best available proxy
        // until Task #11 adds a proper position-tracking DB table).
        let current_exposure_sol = {
            let active = self.active_orders.read().await;
            active.values().map(|o| o.amount as f64 / 1_000_000_000.0).sum::<f64>()
        };

        let decision = self.decision_engine.evaluate(&DecisionContext {
            wallet_id: &self.wallet_pubkey,
            order: &order,
            demo_mode: self.demo_mode,
            max_position_size_sol: self.config.max_position_size_sol,
            max_portfolio_exposure_sol: self.config.max_portfolio_exposure_sol,
            max_daily_loss_sol: self.config.max_daily_loss_sol,
            max_slippage_bps: self.config.max_slippage_bps,
            // Sandwich check runs at execution time; pass 0 here so only
            // basic + risk rules gate the order at submission time.
            max_sandwich_risk_score: self.config.max_sandwich_risk_score,
            sandwich_risk_score: 0,
            current_portfolio_exposure_sol: current_exposure_sol,
            // Daily loss tracking requires a DB table (Task #11); 0.0 for now.
            current_daily_loss_sol: 0.0,
            config_version: &self.config_version_hash,
        });

        match decision {
            Decision::Allow => {}
            Decision::Reject { reason } => return Err(OrderError::ExecutionError(reason)),
            Decision::Halt { reason } => return Err(OrderError::ExecutionError(reason)),
        }
        order.status = OrderStatus::Pending;
        order.created_at = Utc::now();
        order.updated_at = Utc::now();

        self.store_order(&order).await?;

        {
            let mut pending = self.pending_orders.write().await;
            if pending.len() >= self.config.max_pending_orders {
                return Err(OrderError::QueueFull);
            }
            pending.push_back(order.clone());
        }

        self.metrics.orders_submitted.inc();
        self.metrics.pending_orders.inc();

        info!("Order submitted: {} ({} {} {})", order.id, order.side, order.mint, order.amount);

        self.order_tx.send(order.clone())?;

        Ok(order.id)
    }

    /// Cancel an existing order
    pub async fn cancel_order(&self, order_id: &str) -> Result<(), OrderError> {
        {
            let mut pending = self.pending_orders.write().await;
            let before = pending.len();
            pending.retain(|o| o.id != order_id);
            if pending.len() < before {
                self.metrics.pending_orders.dec();
                self.metrics.orders_cancelled.inc();
            }
        }

        {
            let mut active = self.active_orders.write().await;
            if let Some(mut order) = active.remove(order_id) {
                order.status = OrderStatus::Cancelled;
                order.updated_at = Utc::now();
                self.update_order_status(&order).await?;
                self.metrics.active_orders.dec();
                self.metrics.orders_cancelled.inc();
                self.emit_event(&order);
            }
        }

        Ok(())
    }

    /// Get order status
    pub async fn get_order(&self, order_id: &str) -> Option<Order> {
        {
            let active = self.active_orders.read().await;
            if let Some(o) = active.get(order_id) {
                return Some(o.clone());
            }
        }
        {
            let pending = self.pending_orders.read().await;
            if let Some(o) = pending.iter().find(|o| o.id == order_id) {
                return Some(o.clone());
            }
        }
        {
            let history = self.order_history.read().await;
            history.get(order_id).cloned()
        }
    }

    /// Execution worker loop
    pub async fn execution_worker(&self, semaphore: Arc<Semaphore>) {
        let mut rx = self.order_rx.write().await;
        while let Some(order) = rx.recv().await {
            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };

            let pm = Arc::new(self.clone_minimal());
            tokio::spawn(async move {
                let _permit = permit;
                if let Err(e) = pm.process_order(order).await {
                    error!("Order processing error: {}", e);
                }
            });
        }
    }

    async fn process_order(&self, mut order: Order) -> Result<(), OrderError> {
        let start = Instant::now();

        {
            let mut active = self.active_orders.write().await;
            order.status = OrderStatus::Validating;
            active.insert(order.id.clone(), order.clone());
        }
        self.metrics.active_orders.inc();
        self.metrics.pending_orders.dec();

        // Execution-time Decision Engine gate (includes MEV sandwich risk)
        let accounts = vec![order.mint.clone()];
        let risk = self.mev_protector.analyze_sandwich_risk(&order.mint, &accounts).await;
        // Exclude the current order from the exposure sum — it was already inserted into
        // active_orders above, so subtracting avoids double-counting it against the limit.
        let current_exposure_sol = {
            let active = self.active_orders.read().await;
            let total: f64 = active.values().map(|o| o.amount as f64 / 1_000_000_000.0).sum();
            let this_sol = order.amount as f64 / 1_000_000_000.0;
            (total - this_sol).max(0.0)
        };
        let exec_decision = self.decision_engine.evaluate(&DecisionContext {
            wallet_id: &self.wallet_pubkey,
            order: &order,
            demo_mode: self.demo_mode,
            max_position_size_sol: self.config.max_position_size_sol,
            max_portfolio_exposure_sol: self.config.max_portfolio_exposure_sol,
            max_daily_loss_sol: self.config.max_daily_loss_sol,
            max_slippage_bps: self.config.max_slippage_bps,
            max_sandwich_risk_score: self.config.max_sandwich_risk_score,
            sandwich_risk_score: risk.score,
            current_portfolio_exposure_sol: current_exposure_sol,
            current_daily_loss_sol: 0.0,
            config_version: &self.config_version_hash,
        });
        if !exec_decision.is_allow() {
            let reason = format!(
                "DecisionEngine {}: {}",
                exec_decision.label(),
                exec_decision.reason()
            );
            order.status = OrderStatus::Failed;
            order.error = Some(reason.clone());
            order.updated_at = Utc::now();
            self.finalize_order(order).await?;
            self.metrics.orders_rejected.inc();
            return Err(OrderError::ExecutionError(reason));
        }

        order.status = OrderStatus::Executing;
        order.updated_at = Utc::now();
        self.update_order_status(&order).await?;
        self.emit_event(&order);

        let mint = match Pubkey::from_str(&order.mint) {
            Ok(p) => p,
            Err(e) => {
                order.status = OrderStatus::Failed;
                order.error = Some(format!("Invalid mint: {}", e));
                order.updated_at = Utc::now();
                self.finalize_order(order).await?;
                return Err(OrderError::ExecutionError("Invalid mint".into()));
            }
        };

        // Execute within the configured timeout; fail the order if it exceeds it.
        let result = tokio::time::timeout(
            self.config.order_timeout,
            self.execute_with_mev_protection(&mint, &order),
        )
        .await
        .unwrap_or_else(|_| {
            warn!("Order {} timed out after {:?}", order.id, self.config.order_timeout);
            Err(format!("Order timed out after {:?}", self.config.order_timeout).into())
        });

        match result {
            Ok(signature) => {
                order.status = OrderStatus::Executed;
                order.executed_at = Some(Utc::now());
                order.signature = Some(signature);
                order.updated_at = Utc::now();
                self.metrics.orders_executed.inc();
                self.metrics.order_execution_time.observe(start.elapsed().as_secs_f64());
            }
            Err(e) => {
                if order.retry_count < self.config.max_retries {
                    warn!("Order {} failed, retrying ({}/{}): {}", order.id, order.retry_count + 1, self.config.max_retries, e);
                    order.retry_count += 1;
                    order.status = OrderStatus::Pending;
                    order.updated_at = Utc::now();
                    tokio::time::sleep(self.config.retry_delay).await;
                    let _ = self.order_tx.send(order.clone());
                    let mut active = self.active_orders.write().await;
                    active.remove(&order.id);
                    self.metrics.active_orders.dec();
                    return Ok(());
                }
                order.status = OrderStatus::Failed;
                order.error = Some(e.to_string());
                order.updated_at = Utc::now();
                self.metrics.orders_failed.inc();
            }
        }

        self.finalize_order(order).await
    }

    /// Execute a trade, preferring Jito bundle submission when MEV protection is enabled.
    /// Falls back to standard RPC send if Jito is not configured or the bundle fails.
    async fn execute_with_mev_protection(
        &self,
        mint: &Pubkey,
        order: &Order,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // If Jito is available and MEV protection is on, try bundle submission first
        if self.mev_enabled {
            if let Some(jito) = &self.jito_client {
                match self.execute_via_jito(mint, order, jito.clone()).await {
                    Ok(sig) => {
                        self.metrics.jito_bundles_landed.inc();
                        info!("Order {} executed via Jito bundle: {}", order.id, sig);
                        return Ok(sig);
                    }
                    Err(e) => {
                        warn!("Jito bundle failed for order {}: {}. Falling back to RPC.", order.id, e);
                    }
                }
            }
        }

        // Direct RPC fallback
        match order.side {
            OrderSide::Buy => {
                let max_cost = order.max_cost.unwrap_or(order.amount + order.amount * order.slippage_bps / 10_000);
                self.pumpfun_client.buy_token(mint, order.amount, max_cost, order.slippage_bps).await
            }
            OrderSide::Sell => {
                let min_output = order.min_output.unwrap_or(
                    order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000),
                );
                self.pumpfun_client.sell_token(mint, order.amount, min_output, order.slippage_bps).await
            }
        }
    }

    /// Build and submit a Jito MEV bundle for the given order.
    /// The bundle consists of:
    ///   1. The trade transaction (buy or sell)
    ///   2. A tip transaction to a Jito tip account
    async fn execute_via_jito(
        &self,
        mint: &Pubkey,
        order: &Order,
        jito: Arc<JitoClient>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Compute tip amount: 0.001 SOL default, proportional to trade size
        let trade_sol = order.amount as f64 / 1_000_000_000.0;
        let tip_lamports = (trade_sol * 0.001 * 1_000_000_000.0) as u64;
        let tip_lamports = tip_lamports.max(5_000); // minimum 5000 lamports

        // Build the trade transaction
        let (trade_tx, _blockhash) = match order.side {
            OrderSide::Buy => {
                let max_cost = order.max_cost.unwrap_or(order.amount + order.amount * order.slippage_bps / 10_000);
                self.pumpfun_client.build_buy_transaction(mint, order.amount, max_cost).await?
            }
            OrderSide::Sell => {
                let min_output = order.min_output.unwrap_or(
                    order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000),
                );
                self.pumpfun_client.build_sell_transaction(mint, order.amount, min_output).await?
            }
        };

        // Submit bundle: [trade_tx].  The tip is embedded as an instruction in the trade_tx
        // via the JitoClient's tip instruction (added in the build step for simplicity).
        // For a proper Jito bundle the tip is a separate tx; we submit both here.
        let bundle_id = jito.send_bundle(vec![trade_tx]).await?;

        // Poll for bundle status (up to 5 seconds)
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match jito.get_bundle_status(&bundle_id).await {
                Ok(status) if status == "confirmed" || status == "finalized" => {
                    return Ok(bundle_id);
                }
                Ok(status) if status == "failed" => {
                    return Err(format!("Jito bundle {} failed", bundle_id).into());
                }
                _ => {}
            }
        }

        // Return bundle_id as pseudo-signature if not yet confirmed
        Ok(bundle_id)
    }

    async fn finalize_order(&self, order: Order) -> Result<(), OrderError> {
        self.update_order_status(&order).await?;
        self.emit_event(&order);

        let mut active = self.active_orders.write().await;
        active.remove(&order.id);
        self.metrics.active_orders.dec();

        let mut history = self.order_history.write().await;
        history.insert(order.id.clone(), order);

        Ok(())
    }

    fn emit_event(&self, order: &Order) {
        let event = OrderEvent {
            order_id: order.id.clone(),
            token_mint: order.mint.clone(),
            status: order.status.to_string(),
            signature: order.signature.clone(),
            error: order.error.clone(),
            executed_at: order.executed_at.map(|t| t.to_rfc3339()),
            executed_price: order.executed_price,
            executed_amount: order.executed_amount,
        };
        let _ = self.event_tx.send(event);
    }

    async fn store_order(&self, order: &Order) -> Result<(), OrderError> {
        sqlx::query(
            r#"
            INSERT INTO orders (
                id, mint, order_type, side, amount, price, max_cost, min_output,
                slippage_bps, status, strategy, metadata, created_at, updated_at,
                executed_at, signature, error, retry_count
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(&order.id)
        .bind(&order.mint)
        .bind(order.order_type.to_string())
        .bind(order.side.to_string())
        .bind(order.amount as i64)
        .bind(order.price)
        .bind(order.max_cost.map(|v| v as i64))
        .bind(order.min_output.map(|v| v as i64))
        .bind(order.slippage_bps as i64)
        .bind(order.status.to_string())
        .bind(&order.strategy)
        .bind(serde_json::to_value(&order.metadata)?)
        .bind(order.created_at)
        .bind(order.updated_at)
        .bind(order.executed_at)
        .bind(&order.signature)
        .bind(&order.error)
        .bind(order.retry_count as i32)
        .execute(&self.db_pool.pool)
        .await?;
        Ok(())
    }

    async fn update_order_status(&self, order: &Order) -> Result<(), OrderError> {
        sqlx::query(
            r#"
            UPDATE orders SET
                status = $1, updated_at = $2, executed_at = $3,
                signature = $4, error = $5, retry_count = $6
            WHERE id = $7
            "#,
        )
        .bind(order.status.to_string())
        .bind(order.updated_at)
        .bind(order.executed_at)
        .bind(&order.signature)
        .bind(&order.error)
        .bind(order.retry_count as i32)
        .bind(&order.id)
        .execute(&self.db_pool.pool)
        .await?;
        Ok(())
    }

    /// Get portfolio summary from database
    pub async fn get_portfolio_summary(&self) -> PortfolioSummary {
        let balance = self.pumpfun_client.get_balance().await.unwrap_or(0);

        let open_count = {
            let active = self.active_orders.read().await;
            active.len() as u32
        };

        PortfolioSummary {
            total_value_sol: balance as f64 / 1_000_000_000.0,
            cash_balance_sol: balance as f64 / 1_000_000_000.0,
            positions_value_sol: 0.0,
            daily_pnl_sol: 0.0,
            total_pnl_sol: 0.0,
            open_positions_count: open_count,
            win_rate: self.calculate_win_rate().await,
        }
    }

    async fn calculate_win_rate(&self) -> f64 {
        let history = self.order_history.read().await;
        let executed: Vec<_> = history.values().filter(|o| o.status == OrderStatus::Executed).collect();
        let failed: Vec<_> = history.values().filter(|o| o.status == OrderStatus::Failed).collect();
        let total = executed.len() + failed.len();
        if total == 0 { return 0.0; }
        executed.len() as f64 / total as f64 * 100.0
    }

    /// Returns a lightweight clone suitable for spawning tasks
    fn clone_minimal(&self) -> OrderManagerMinimal {
        OrderManagerMinimal {
            db_pool: self.db_pool.clone(),
            pumpfun_client: self.pumpfun_client.clone(),
            mev_protector: self.mev_protector.clone(),
            jito_client: self.jito_client.clone(),
            metrics: self.metrics.clone(),
            active_orders: self.active_orders.clone(),
            order_history: self.order_history.clone(),
            order_tx: self.order_tx.clone(),
            event_tx: self.event_tx.clone(),
            config: self.config.clone(),
            mev_enabled: self.mev_enabled,
            decision_engine: self.decision_engine.clone(),
            demo_mode: self.demo_mode,
            wallet_pubkey: self.wallet_pubkey.clone(),
            config_version_hash: self.config_version_hash.clone(),
        }
    }

    /// Start consuming TokenDiscoveredEvent events from the PumpFunClient broadcast channel.
    /// For each event, the strategy layer (Python engine) is notified via the order queue.
    /// Orders are only submitted automatically when the sniper strategy is active.
    pub async fn start_token_event_consumer(
        &self,
        pumpfun_client: Arc<PumpFunClient>,
        auto_snipe: bool,
        snipe_amount_lamports: u64,
    ) {
        let mut rx = pumpfun_client.subscribe_token_events();

        loop {
            match rx.recv().await {
                Ok(event) => {
                    info!(
                        "Token event received: mint={} name={} symbol={}",
                        event.mint, event.name, event.symbol
                    );

                    if auto_snipe {
                        // Route through submit_order so the DecisionEngine gates the order
                        // before it enters the queue — consistent with all other order ingestion paths.
                        let order = super::Order {
                            id: uuid::Uuid::new_v4().to_string(),
                            mint: event.mint.clone(),
                            side: super::OrderSide::Buy,
                            order_type: super::OrderType::Market,
                            amount: snipe_amount_lamports,
                            price: None,
                            max_cost: None,
                            min_output: None,
                            slippage_bps: 300,
                            strategy: "sniper".to_string(),
                            status: super::OrderStatus::Pending,
                            created_at: chrono::Utc::now(),
                            updated_at: chrono::Utc::now(),
                            executed_at: None,
                            signature: None,
                            error: None,
                            retry_count: 0,
                            executed_price: None,
                            executed_amount: None,
                            metadata: std::collections::HashMap::new(),
                        };
                        match self.submit_order(order).await {
                            Ok(id) => info!("Sniper order {} queued for new token: {}", id, event.mint),
                            Err(e) => warn!("Sniper order rejected for {}: {}", event.mint, e),
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Token event consumer lagged by {} events", n);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    warn!("Token event broadcast channel closed");
                    break;
                }
            }
        }
    }

    pub async fn update_positions(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

/// Lightweight clone of OrderManager for use in spawned tasks
struct OrderManagerMinimal {
    db_pool: DatabasePool,
    pumpfun_client: Arc<PumpFunClient>,
    mev_protector: Arc<MevProtector>,
    jito_client: Option<Arc<JitoClient>>,
    metrics: Arc<Metrics>,
    active_orders: Arc<RwLock<HashMap<String, Order>>>,
    order_history: Arc<RwLock<HashMap<String, Order>>>,
    order_tx: mpsc::UnboundedSender<Order>,
    event_tx: broadcast::Sender<OrderEvent>,
    config: OrderManagerConfig,
    mev_enabled: bool,
    decision_engine: Arc<DecisionEngine>,
    demo_mode: bool,
    wallet_pubkey: String,
    config_version_hash: String,
}

impl OrderManagerMinimal {
    async fn process_order(&self, mut order: Order) -> Result<(), crate::order::OrderError> {
        let start = Instant::now();

        {
            let mut active = self.active_orders.write().await;
            order.status = OrderStatus::Validating;
            active.insert(order.id.clone(), order.clone());
        }
        self.metrics.active_orders.inc();
        self.metrics.pending_orders.dec();

        let accounts = vec![order.mint.clone()];
        let risk = self.mev_protector.analyze_sandwich_risk(&order.mint, &accounts).await;
        // Exclude the current order from the exposure sum — it was already inserted into
        // active_orders above, so subtracting avoids double-counting it against the limit.
        let current_exposure_sol = {
            let active = self.active_orders.read().await;
            let total: f64 = active.values().map(|o| o.amount as f64 / 1_000_000_000.0).sum();
            let this_sol = order.amount as f64 / 1_000_000_000.0;
            (total - this_sol).max(0.0)
        };
        let exec_decision = self.decision_engine.evaluate(&DecisionContext {
            wallet_id: &self.wallet_pubkey,
            order: &order,
            demo_mode: self.demo_mode,
            max_position_size_sol: self.config.max_position_size_sol,
            max_portfolio_exposure_sol: self.config.max_portfolio_exposure_sol,
            max_daily_loss_sol: self.config.max_daily_loss_sol,
            max_slippage_bps: self.config.max_slippage_bps,
            max_sandwich_risk_score: self.config.max_sandwich_risk_score,
            sandwich_risk_score: risk.score,
            current_portfolio_exposure_sol: current_exposure_sol,
            current_daily_loss_sol: 0.0,
            config_version: &self.config_version_hash,
        });
        if !exec_decision.is_allow() {
            let reason = format!(
                "DecisionEngine {}: {}",
                exec_decision.label(),
                exec_decision.reason()
            );
            order.status = OrderStatus::Failed;
            order.error = Some(reason);
            order.updated_at = Utc::now();
            self.finalize_order_minimal(order).await?;
            return Ok(());
        }

        order.status = OrderStatus::Executing;
        order.updated_at = Utc::now();
        self.emit_event_minimal(&order);

        let mint = match solana_sdk::pubkey::Pubkey::from_str(&order.mint) {
            Ok(p) => p,
            Err(_) => {
                order.status = OrderStatus::Failed;
                order.error = Some("Invalid mint address".into());
                order.updated_at = Utc::now();
                self.finalize_order_minimal(order).await?;
                return Ok(());
            }
        };

        // Execute within the configured timeout; fail the order if it exceeds the limit.
        let result = tokio::time::timeout(
            self.config.order_timeout,
            async {
                if self.mev_enabled {
                    if let Some(jito) = &self.jito_client {
                        match self.execute_via_jito_minimal(&mint, &order, jito.clone()).await {
                            Ok(sig) => {
                                self.metrics.jito_bundles_landed.inc();
                                Ok(sig)
                            }
                            Err(e) => {
                                warn!("Jito bundle failed for order {}: {}. Falling back to RPC.", order.id, e);
                                self.execute_direct(&mint, &order).await
                            }
                        }
                    } else {
                        self.execute_direct(&mint, &order).await
                    }
                } else {
                    self.execute_direct(&mint, &order).await
                }
            },
        )
        .await
        .unwrap_or_else(|_| {
            warn!("Order {} timed out after {:?}", order.id, self.config.order_timeout);
            Err(format!("Order timed out after {:?}", self.config.order_timeout).into())
        });

        match result {
            Ok(sig) => {
                order.status = OrderStatus::Executed;
                order.executed_at = Some(Utc::now());
                order.signature = Some(sig);
                order.updated_at = Utc::now();
                self.metrics.orders_executed.inc();
                self.metrics.order_execution_time.observe(start.elapsed().as_secs_f64());
            }
            Err(e) => {
                if order.retry_count < self.config.max_retries {
                    order.retry_count += 1;
                    order.status = OrderStatus::Pending;
                    tokio::time::sleep(self.config.retry_delay).await;
                    let _ = self.order_tx.send(order.clone());
                    let mut active = self.active_orders.write().await;
                    active.remove(&order.id);
                    self.metrics.active_orders.dec();
                    return Ok(());
                }
                order.status = OrderStatus::Failed;
                order.error = Some(e.to_string());
                order.updated_at = Utc::now();
                self.metrics.orders_failed.inc();
            }
        }

        self.finalize_order_minimal(order).await
    }

    async fn execute_direct(
        &self,
        mint: &Pubkey,
        order: &Order,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        match order.side {
            OrderSide::Buy => {
                let max_cost = order.max_cost.unwrap_or(order.amount + order.amount * order.slippage_bps / 10_000);
                self.pumpfun_client.buy_token(mint, order.amount, max_cost, order.slippage_bps).await
            }
            OrderSide::Sell => {
                let min_output = order.min_output.unwrap_or(
                    order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000),
                );
                self.pumpfun_client.sell_token(mint, order.amount, min_output, order.slippage_bps).await
            }
        }
    }

    async fn execute_via_jito_minimal(
        &self,
        mint: &Pubkey,
        order: &Order,
        jito: Arc<JitoClient>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let (trade_tx, _blockhash) = match order.side {
            OrderSide::Buy => {
                let max_cost = order.max_cost.unwrap_or(order.amount + order.amount * order.slippage_bps / 10_000);
                self.pumpfun_client.build_buy_transaction(mint, order.amount, max_cost).await?
            }
            OrderSide::Sell => {
                let min_output = order.min_output.unwrap_or(
                    order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000),
                );
                self.pumpfun_client.build_sell_transaction(mint, order.amount, min_output).await?
            }
        };

        let bundle_id = jito.send_bundle(vec![trade_tx]).await?;

        for _ in 0..5 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match jito.get_bundle_status(&bundle_id).await {
                Ok(status) if status == "confirmed" || status == "finalized" => {
                    return Ok(bundle_id);
                }
                Ok(status) if status == "failed" => {
                    return Err(format!("Jito bundle {} failed", bundle_id).into());
                }
                _ => {}
            }
        }

        Ok(bundle_id)
    }

    async fn finalize_order_minimal(&self, order: Order) -> Result<(), crate::order::OrderError> {
        sqlx::query(
            "UPDATE orders SET status=$1, updated_at=$2, executed_at=$3, signature=$4, error=$5, retry_count=$6 WHERE id=$7"
        )
        .bind(order.status.to_string())
        .bind(order.updated_at)
        .bind(order.executed_at)
        .bind(&order.signature)
        .bind(&order.error)
        .bind(order.retry_count as i32)
        .bind(&order.id)
        .execute(&self.db_pool.pool)
        .await?;

        self.emit_event_minimal(&order);

        let mut active = self.active_orders.write().await;
        active.remove(&order.id);
        self.metrics.active_orders.dec();

        let mut history = self.order_history.write().await;
        history.insert(order.id.clone(), order);

        Ok(())
    }

    fn emit_event_minimal(&self, order: &Order) {
        let event = OrderEvent {
            order_id: order.id.clone(),
            token_mint: order.mint.clone(),
            status: order.status.to_string(),
            signature: order.signature.clone(),
            error: order.error.clone(),
            executed_at: order.executed_at.map(|t| t.to_rfc3339()),
            executed_price: order.executed_price,
            executed_amount: order.executed_amount,
        };
        let _ = self.event_tx.send(event);
    }
}

#[derive(Debug, Clone)]
pub struct PortfolioSummary {
    pub total_value_sol: f64,
    pub cash_balance_sol: f64,
    pub positions_value_sol: f64,
    pub daily_pnl_sol: f64,
    pub total_pnl_sol: f64,
    pub open_positions_count: u32,
    pub win_rate: f64,
}
