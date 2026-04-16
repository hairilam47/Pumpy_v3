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

use crate::database::{self, DatabasePool};
use crate::decision::{Decision, DecisionContext, DecisionEngine};
use crate::metrics::Metrics;
use crate::mev::{MevProtector, JitoClient};
use crate::pumpfun::PumpFunClient;
use super::{Order, OrderError, OrderSide, OrderStatus, OrderType};

/// Returns `true` when an execution error is worth retrying (transient / network errors).
///
/// Deterministic Solana failures (bad parameters, insufficient balance, duplicate
/// transaction) fail fast without consuming the retry budget.  Transient errors
/// (network timeouts, RPC pressure, blockhash expiry) are allowed to retry.
///
/// NOTE: "blockhash not found" is intentionally treated as *retriable* — under
/// RPC pressure a valid blockhash can expire before the tx reaches a leader;
/// a fresh retry fetches a new blockhash and usually succeeds.
fn is_retriable_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    !lower.contains("invalid mint")
        && !lower.contains("insufficient funds")
        && !lower.contains("insufficient lamports")
        && !lower.contains("already processed")
}

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
            retry_delay: Duration::from_millis(200),
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
    /// Logical wallet_id (e.g. "wallet_001") matching wallet_registry.wallet_id.
    /// Used for all DB writes (pause_wallet, get_wallet_status) so that updates
    /// land on the correct row — wallet_pubkey is the Solana address, not the PK.
    wallet_id: String,
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
        wallet_id: String,
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
            wallet_id,
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
            // Bonding curve not fetched at submit time — price-impact check is
            // deferred to the execution-gate where we have a live pool snapshot.
            bonding_curve_params: None,
        });

        // Self-healing resume: if the decision engine is still in auto-paused latch
        // but the DB wallet status has been manually flipped back to "enabled" by an
        // operator, reset the in-memory state so orders can flow again.
        if self.decision_engine.is_auto_paused() {
            let db_status = database::get_wallet_status(&self.db_pool.pool, &self.wallet_id).await;
            if db_status.as_deref() == Some("enabled") {
                self.decision_engine.reset_pause();
                info!(wallet_id = %self.wallet_id, "DecisionEngine auto-pause reset: wallet resumed in DB");
            }
        }

        if self.decision_engine.take_needs_db_pause() {
            let pool = self.db_pool.pool.clone();
            let wid = self.wallet_id.clone();
            let halt_reason = match &decision {
                Decision::Halt { reason } => reason.clone(),
                _ => "consecutive_reject threshold exceeded".to_string(),
            };
            let reject_count = self.decision_engine.consecutive_rejects_count();
            tokio::spawn(async move {
                database::pause_wallet(&pool, &wid, &halt_reason, reject_count).await;
            });
        }

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

        // Parse mint early — required before the bonding curve fetch that feeds
        // the execution-gate DecisionEngine call.
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

        // Execution-time Decision Engine gate (includes MEV sandwich risk)
        let accounts = vec![order.mint.clone()];
        let risk = self.mev_protector.analyze_sandwich_risk(&order.mint, &accounts).await;

        // Fetch live bonding-curve params for dynamic slippage validation in
        // DecisionEngine.  Best-effort: on failure the price-impact check is
        // skipped and execution falls back to static slippage.
        let bonding_curve_params =
            self.pumpfun_client.get_bonding_curve_params(&mint).await.ok();

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
            bonding_curve_params: bonding_curve_params.as_ref(),
        });
        if self.decision_engine.take_needs_db_pause() {
            let pool = self.db_pool.pool.clone();
            let wid = self.wallet_id.clone();
            let halt_reason = match &exec_decision {
                Decision::Halt { reason } => reason.clone(),
                _ => "consecutive_reject threshold exceeded (execution gate)".to_string(),
            };
            let reject_count = self.decision_engine.consecutive_rejects_count();
            tokio::spawn(async move {
                database::pause_wallet(&pool, &wid, &halt_reason, reject_count).await;
            });
        }

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

        // Execute within the configured timeout; fail the order if it exceeds it.
        // Pass the already-fetched bonding curve params to avoid a redundant RPC call.
        let result = tokio::time::timeout(
            self.config.order_timeout,
            self.execute_with_mev_protection(&mint, &order, bonding_curve_params),
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
                let err_str = e.to_string();
                if order.retry_count < self.config.max_retries && is_retriable_error(&err_str) {
                    order.retry_count += 1;
                    // Exponential backoff: base_delay * 2^(attempt-1)
                    let backoff = self.config.retry_delay
                        * 2u32.pow(order.retry_count.saturating_sub(1));
                    warn!(
                        order_id = %order.id,
                        attempt = order.retry_count,
                        max_retries = self.config.max_retries,
                        backoff_ms = backoff.as_millis(),
                        reason = %err_str,
                        "Order failed, retrying with exponential backoff"
                    );
                    order.status = OrderStatus::Pending;
                    order.updated_at = Utc::now();
                    tokio::time::sleep(backoff).await;
                    let _ = self.order_tx.send(order.clone());
                    let mut active = self.active_orders.write().await;
                    active.remove(&order.id);
                    self.metrics.active_orders.dec();
                    return Ok(());
                }
                warn!(
                    order_id = %order.id,
                    attempt = order.retry_count + 1,
                    reason = %err_str,
                    retriable = is_retriable_error(&err_str),
                    "Order failed — not retrying"
                );
                order.status = OrderStatus::Failed;
                order.error = Some(err_str);
                order.updated_at = Utc::now();
                self.metrics.orders_failed.inc();
            }
        }

        self.finalize_order(order).await
    }

    /// Fetch bonding curve params and compute dynamic slippage bounds.
    /// Falls back to static slippage calculation if the RPC fetch fails.
    async fn compute_slippage_bounds(&self, mint: &Pubkey, order: &Order) -> (u64, u64) {
        match self.pumpfun_client.get_bonding_curve_params(mint).await {
            Ok(bc) => match order.side {
                OrderSide::Buy => {
                    let (_, impact_bps, dyn_max_cost) =
                        bc.compute_buy_params(order.amount, self.config.max_slippage_bps);
                    let max_cost = order.max_cost.unwrap_or(dyn_max_cost);
                    info!(
                        order_id = %order.id,
                        price_impact_bps = impact_bps,
                        max_sol_cost = max_cost,
                        "dynamic buy slippage from bonding curve"
                    );
                    (max_cost, 0)
                }
                OrderSide::Sell => {
                    let (_, impact_bps, dyn_min_output) =
                        bc.compute_sell_params(order.amount, self.config.max_slippage_bps);
                    let min_output = order.min_output.unwrap_or(dyn_min_output);
                    info!(
                        order_id = %order.id,
                        price_impact_bps = impact_bps,
                        min_sol_output = min_output,
                        "dynamic sell slippage from bonding curve"
                    );
                    (0, min_output)
                }
            },
            Err(e) => {
                warn!(
                    order_id = %order.id,
                    error = %e,
                    "bonding curve fetch failed; using static slippage"
                );
                let max_cost = order.max_cost.unwrap_or(
                    order.amount + order.amount * order.slippage_bps / 10_000,
                );
                let min_output = order.min_output.unwrap_or(
                    order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000),
                );
                (max_cost, min_output)
            }
        }
    }

    /// Execute a trade, preferring Jito bundle submission when MEV protection is enabled.
    /// On Jito failure (both internal retry attempts), falls back to direct RPC.
    /// Slippage bounds are resolved from the pre-fetched bonding curve snapshot
    /// (passed by the caller) to avoid a redundant RPC round-trip.
    async fn execute_with_mev_protection(
        &self,
        mint: &Pubkey,
        order: &Order,
        bonding_curve_params: Option<crate::pumpfun::bonding_curve::BondingCurveParams>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Resolve slippage bounds from the pre-fetched snapshot (or static fallback).
        let (dynamic_max_cost, dynamic_min_output) = Self::resolve_slippage_bounds(
            order, bonding_curve_params.as_ref(),
        );

        if self.mev_enabled {
            if let Some(jito) = &self.jito_client {
                match self.execute_via_jito(mint, order, jito.clone(), dynamic_max_cost, dynamic_min_output).await {
                    Ok(sig) => {
                        self.metrics.jito_bundles_landed.inc();
                        info!(order_id = %order.id, sig = %sig, "executed via Jito bundle");
                        return Ok(sig);
                    }
                    Err(e) => {
                        warn!(
                            order_id = %order.id,
                            error = %e,
                            "Jito bundle failed (both attempts); falling back to RPC"
                        );
                    }
                }
            }
        }

        // Direct RPC fallback — use pre-computed bounds.
        info!(
            order_id = %order.id,
            max_sol_cost = dynamic_max_cost,
            min_sol_output = dynamic_min_output,
            "Dynamic slippage bounds applied for RPC execution"
        );
        match order.side {
            OrderSide::Buy => {
                self.pumpfun_client.buy_token(mint, order.amount, dynamic_max_cost, order.slippage_bps).await
            }
            OrderSide::Sell => {
                self.pumpfun_client.sell_token(mint, order.amount, dynamic_min_output, order.slippage_bps).await
            }
        }
    }

    /// Compute `(max_sol_cost, min_sol_output)` from a bonding-curve snapshot when
    /// available, falling back to static slippage when the snapshot is absent.
    fn resolve_slippage_bounds(
        order: &Order,
        bonding_curve_params: Option<&crate::pumpfun::bonding_curve::BondingCurveParams>,
    ) -> (u64, u64) {
        if let Some(params) = bonding_curve_params {
            params.calculate_price_impact(order.amount, order.slippage_bps)
        } else {
            let mc = order.amount + order.amount * order.slippage_bps / 10_000;
            let mo = order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000);
            (mc, mo)
        }
    }

    /// Build and submit a Jito MEV bundle for the given order.
    /// `dynamic_max_cost` / `dynamic_min_output` are pre-computed slippage bounds from the
    /// bonding curve snapshot. On `send_bundle()` error or bundle status "failed", the engine
    /// waits 2 seconds and retries once before returning an error (which causes the
    /// caller to fall back to direct RPC submission).
    async fn execute_via_jito(
        &self,
        mint: &Pubkey,
        order: &Order,
        jito: Arc<JitoClient>,
        dynamic_max_cost: u64,
        dynamic_min_output: u64,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let max_cost = order.max_cost.unwrap_or(dynamic_max_cost);
        let min_output = order.min_output.unwrap_or(dynamic_min_output);

        // Dynamic tip: read floor/ceiling/percent from bot_config at order time.
        // Falls back to safe defaults when a key is absent or unparseable.
        let tip_percent = database::get_config_value(&self.db_pool.pool, "JITO_TIP_PERCENT")
            .await
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.001);
        let tip_floor = database::get_config_value(&self.db_pool.pool, "JITO_TIP_FLOOR_LAMPORTS")
            .await
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5_000);
        let tip_ceiling = database::get_config_value(&self.db_pool.pool, "JITO_TIP_CEILING_LAMPORTS")
            .await
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(10_000_000);
        let tip_lamports = JitoClient::compute_dynamic_tip(order.amount, tip_percent, tip_floor, tip_ceiling);

        // Build the tip instruction once — a randomly-selected Jito tip account is chosen
        // here and reused for every attempt so all retries target the same account.
        let payer = self.pumpfun_client.pubkey();
        let tip_instruction = jito.create_tip_instruction(&payer, tip_lamports);
        if tip_instruction.is_none() {
            warn!(order_id = %order.id, "No Jito tip accounts available — bundle will land without tip");
        }

        // Pre-submission simulation: reject orders likely to fail on-chain before they
        // consume a Jito bundle slot. Simulation failure is not transient so we do not
        // retry — run it once before the submission retry loop.
        let sim_enabled = JitoClient::sim_enabled_from_str(
            database::get_config_value(&self.db_pool.pool, "JITO_SIMULATION_ENABLED").await,
        );

        // Only build the simulation transaction and run the gate when simulation is enabled.
        // Keeping the build inside the guard means a construction failure cannot block
        // execution when JITO_SIMULATION_ENABLED=false.
        if sim_enabled {
            let sim_tx = match order.side {
                OrderSide::Buy => {
                    self.pumpfun_client.build_buy_transaction_with_tip(mint, order.amount, max_cost, tip_instruction.clone()).await?.0
                }
                OrderSide::Sell => {
                    self.pumpfun_client.build_sell_transaction_with_tip(mint, order.amount, min_output, tip_instruction.clone()).await?.0
                }
            };
            jito.execute_simulation_gate(&sim_tx, &order.id).await?;
            info!(order_id = %order.id, "Pre-submission simulation passed");
        }

        // Attempt Jito bundle submission — retries once on any rejection before giving up.
        // Rejection includes both send_bundle() transport errors and bundle status "failed".
        for jito_attempt in 0u32..2 {
            if jito_attempt > 0 {
                warn!(
                    order_id = %order.id,
                    jito_attempt,
                    "Jito bundle rejected, retrying after 2 s"
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            // Rebuild the transaction each attempt to obtain a fresh blockhash.
            // The tip instruction is cloned from the one created above so the same
            // tip account is used across retries.
            let (trade_tx, _blockhash) = match order.side {
                OrderSide::Buy => {
                    self.pumpfun_client.build_buy_transaction_with_tip(mint, order.amount, max_cost, tip_instruction.clone()).await?
                }
                OrderSide::Sell => {
                    self.pumpfun_client.build_sell_transaction_with_tip(mint, order.amount, min_output, tip_instruction.clone()).await?
                }
            };

            // send_bundle() failure is treated as a rejection — retry once then give up.
            let bundle_id = match jito.send_bundle(vec![trade_tx]).await {
                Ok(id) => id,
                Err(e) => {
                    warn!(order_id = %order.id, jito_attempt, reason = %e, tip_lamports, "Jito send_bundle error (treated as rejection)");
                    continue;
                }
            };

            info!(order_id = %order.id, bundle_id = %bundle_id, jito_attempt, tip_lamports, "Jito bundle submitted (tip instruction included)");

            // Poll for bundle status (up to 5 seconds).
            let mut bundle_failed = false;
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                match jito.get_bundle_status(&bundle_id).await {
                    Ok(status) if status == "confirmed" || status == "finalized" => {
                        info!(order_id = %order.id, bundle_id = %bundle_id, jito_attempt, "Jito bundle landed");
                        return Ok(bundle_id);
                    }
                    Ok(status) if status == "failed" => {
                        bundle_failed = true;
                        break;
                    }
                    _ => {}
                }
            }

            if !bundle_failed {
                // Polling timed out — return bundle_id as pseudo-signature for optimistic tracking.
                return Ok(bundle_id);
            }
            // bundle_failed=true → loop to retry (jito_attempt 0→1) or fall through to Err
        }

        Err(format!("Jito bundle failed after 2 attempt(s) for order {}", order.id).into())
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
                executed_at, signature, error, retry_count, client_order_id
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
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
        .bind(&order.client_order_id)
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
            wallet_id: self.wallet_id.clone(),
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
                            client_order_id: Some(uuid::Uuid::new_v4()),
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
    /// Logical wallet_id for DB writes. Must match wallet_registry.wallet_id.
    wallet_id: String,
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

        // Parse mint early — required before the bonding curve fetch that feeds
        // the execution-gate DecisionEngine call.
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

        let accounts = vec![order.mint.clone()];
        let risk = self.mev_protector.analyze_sandwich_risk(&order.mint, &accounts).await;

        // Fetch live bonding-curve params for dynamic slippage validation in
        // DecisionEngine.  Best-effort: on failure the price-impact check is
        // skipped and execution falls back to static slippage.
        let bonding_curve_params =
            self.pumpfun_client.get_bonding_curve_params(&mint).await.ok();

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
            bonding_curve_params: bonding_curve_params.as_ref(),
        });
        if self.decision_engine.take_needs_db_pause() {
            let pool = self.db_pool.pool.clone();
            let wid = self.wallet_id.clone();
            let halt_reason = match &exec_decision {
                Decision::Halt { reason } => reason.clone(),
                _ => "consecutive_reject threshold exceeded (execution gate)".to_string(),
            };
            let reject_count = self.decision_engine.consecutive_rejects_count();
            tokio::spawn(async move {
                database::pause_wallet(&pool, &wid, &halt_reason, reject_count).await;
            });
        }

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

        // Compute slippage bounds once from the already-fetched snapshot so we
        // don't re-fetch inside the execution functions.
        let (dynamic_max_cost, dynamic_min_output) = if let Some(ref params) = bonding_curve_params {
            params.calculate_price_impact(order.amount, order.slippage_bps)
        } else {
            let mc = order.amount + order.amount * order.slippage_bps / 10_000;
            let mo = order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000);
            (mc, mo)
        };
        let max_cost = order.max_cost.unwrap_or(dynamic_max_cost);
        let min_output = order.min_output.unwrap_or(dynamic_min_output);

        // Execute within the configured timeout; fail the order if it exceeds the limit.
        // mint was already parsed early; no second Pubkey::from_str needed here.
        let result = tokio::time::timeout(
            self.config.order_timeout,
            async {
                if self.mev_enabled {
                    if let Some(jito) = &self.jito_client {
                        match self.execute_via_jito_minimal(&mint, &order, jito.clone(), max_cost, min_output).await {
                            Ok(sig) => {
                                self.metrics.jito_bundles_landed.inc();
                                Ok(sig)
                            }
                            Err(e) => {
                                warn!("Jito bundle failed for order {}: {}. Falling back to RPC.", order.id, e);
                                self.execute_direct(&mint, &order, max_cost, min_output).await
                            }
                        }
                    } else {
                        self.execute_direct(&mint, &order, max_cost, min_output).await
                    }
                } else {
                    self.execute_direct(&mint, &order, max_cost, min_output).await
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
                let err_str = e.to_string();
                if order.retry_count < self.config.max_retries && is_retriable_error(&err_str) {
                    let backoff = self.config.retry_delay * 2u32.pow(order.retry_count);
                    warn!(
                        order_id = %order.id,
                        attempt = order.retry_count + 1,
                        max_attempts = self.config.max_retries + 1,
                        reason = %err_str,
                        backoff_ms = backoff.as_millis(),
                        "Order execution failed, scheduling retry with exponential backoff"
                    );
                    order.retry_count += 1;
                    order.status = OrderStatus::Pending;
                    tokio::time::sleep(backoff).await;
                    let _ = self.order_tx.send(order.clone());
                    let mut active = self.active_orders.write().await;
                    active.remove(&order.id);
                    self.metrics.active_orders.dec();
                    return Ok(());
                }
                warn!(
                    order_id = %order.id,
                    attempt = order.retry_count + 1,
                    reason = %err_str,
                    retriable = is_retriable_error(&err_str),
                    "Order failed — not retrying"
                );
                order.status = OrderStatus::Failed;
                order.error = Some(err_str);
                order.updated_at = Utc::now();
                self.metrics.orders_failed.inc();
            }
        }

        self.finalize_order_minimal(order).await
    }

    /// Submit a trade directly via RPC using pre-computed slippage bounds.
    async fn execute_direct(
        &self,
        mint: &Pubkey,
        order: &Order,
        max_cost: u64,
        min_output: u64,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        match order.side {
            OrderSide::Buy => {
                self.pumpfun_client.buy_token(mint, order.amount, max_cost, order.slippage_bps).await
            }
            OrderSide::Sell => {
                self.pumpfun_client.sell_token(mint, order.amount, min_output, order.slippage_bps).await
            }
        }
    }

    /// Build and submit a Jito MEV bundle with pre-computed slippage bounds.
    ///
    /// On bundle rejection — whether from `send_bundle()` returning an error
    /// **or** from the bundle status API reporting "failed" — the engine waits
    /// 2 seconds and retries once before returning an error (which causes the
    /// caller to fall back to direct RPC submission).
    async fn execute_via_jito_minimal(
        &self,
        mint: &Pubkey,
        order: &Order,
        jito: Arc<JitoClient>,
        max_cost: u64,
        min_output: u64,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Dynamic tip: read floor/ceiling/percent from bot_config at order time.
        let tip_percent = database::get_config_value(&self.db_pool.pool, "JITO_TIP_PERCENT")
            .await
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.001);
        let tip_floor = database::get_config_value(&self.db_pool.pool, "JITO_TIP_FLOOR_LAMPORTS")
            .await
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5_000);
        let tip_ceiling = database::get_config_value(&self.db_pool.pool, "JITO_TIP_CEILING_LAMPORTS")
            .await
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(10_000_000);
        let tip_lamports = JitoClient::compute_dynamic_tip(order.amount, tip_percent, tip_floor, tip_ceiling);

        // Build the tip instruction once — a randomly-selected Jito tip account is chosen
        // here and reused for every attempt so all retries target the same account.
        let payer = self.pumpfun_client.pubkey();
        let tip_instruction = jito.create_tip_instruction(&payer, tip_lamports);
        if tip_instruction.is_none() {
            warn!(order_id = %order.id, "No Jito tip accounts available — bundle will land without tip");
        }

        // Pre-submission simulation: reject orders likely to fail on-chain before they
        // consume a Jito bundle slot. Simulation failure is not transient so we run
        // it once before the retry loop.
        let sim_enabled = JitoClient::sim_enabled_from_str(
            database::get_config_value(&self.db_pool.pool, "JITO_SIMULATION_ENABLED").await,
        );

        // Only build the simulation transaction and run the gate when simulation is enabled.
        if sim_enabled {
            let sim_tx = match order.side {
                OrderSide::Buy => {
                    self.pumpfun_client.build_buy_transaction_with_tip(mint, order.amount, max_cost, tip_instruction.clone()).await?.0
                }
                OrderSide::Sell => {
                    self.pumpfun_client.build_sell_transaction_with_tip(mint, order.amount, min_output, tip_instruction.clone()).await?.0
                }
            };
            jito.execute_simulation_gate(&sim_tx, &order.id).await?;
            info!(order_id = %order.id, "Pre-submission simulation passed");
        }

        // Attempt Jito bundle submission — retries once on any rejection before giving up.
        // Rejection includes both send_bundle() transport errors and bundle status "failed".
        for jito_attempt in 0u32..2 {
            if jito_attempt > 0 {
                warn!(
                    order_id = %order.id,
                    jito_attempt,
                    "Jito bundle rejected, retrying after 2 s"
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            // Rebuild the transaction each attempt to obtain a fresh blockhash.
            // Tip instruction is cloned from the one created above (same tip account).
            let (trade_tx, _blockhash) = match order.side {
                OrderSide::Buy => {
                    self.pumpfun_client.build_buy_transaction_with_tip(mint, order.amount, max_cost, tip_instruction.clone()).await?
                }
                OrderSide::Sell => {
                    self.pumpfun_client.build_sell_transaction_with_tip(mint, order.amount, min_output, tip_instruction.clone()).await?
                }
            };

            // send_bundle() failure is treated as a rejection — retry once then give up.
            let bundle_id = match jito.send_bundle(vec![trade_tx]).await {
                Ok(id) => id,
                Err(e) => {
                    warn!(order_id = %order.id, jito_attempt, reason = %e, tip_lamports, "Jito send_bundle error (treated as rejection)");
                    continue; // will sleep 2 s at the start of the next iteration (or exit loop)
                }
            };

            let mut bundle_failed = false;
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                match jito.get_bundle_status(&bundle_id).await {
                    Ok(status) if status == "confirmed" || status == "finalized" => {
                        info!(order_id = %order.id, bundle_id = %bundle_id, jito_attempt, "Jito bundle landed");
                        return Ok(bundle_id);
                    }
                    Ok(status) if status == "failed" => {
                        bundle_failed = true;
                        break;
                    }
                    _ => {}
                }
            }

            if !bundle_failed {
                return Ok(bundle_id);
            }
        }

        Err(format!("Jito bundle failed after 2 attempt(s) for order {}", order.id).into())
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

#[cfg(test)]
mod tests {
    use crate::mev::jito::JitoClient;
    use solana_sdk::{hash::Hash, signature::Keypair, signer::Signer, system_instruction};
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

    fn dummy_tx() -> solana_sdk::transaction::Transaction {
        let payer = Keypair::new();
        let ix = system_instruction::transfer(&payer.pubkey(), &payer.pubkey(), 0);
        solana_sdk::transaction::Transaction::new_signed_with_payer(
            &[ix], Some(&payer.pubkey()), &[&payer], Hash::default(),
        )
    }

    async fn mock_sim_rpc(body: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = l.accept().await {
                let mut buf = vec![0u8; 4096];
                let _ = s.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body);
                let _ = s.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://{}", addr)
    }

    async fn mock_bundle_endpoint(body: &'static str, called: Arc<AtomicBool>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = l.accept().await {
                called.store(true, Ordering::SeqCst);
                let mut buf = vec![0u8; 4096];
                let _ = s.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(), body);
                let _ = s.write_all(resp.as_bytes()).await;
            }
        });
        format!("http://{}", addr)
    }

    // Replicates the manager's exact gate pattern: if sim_enabled { gate()? } send_bundle()
    async fn sim_gate_then_bundle(
        jito: &JitoClient,
        tx: solana_sdk::transaction::Transaction,
        sim_enabled: bool,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if sim_enabled {
            jito.execute_simulation_gate(&tx, "order-test").await?;
        }
        jito.send_bundle(vec![tx]).await
    }

    // sim fails → execute_simulation_gate returns Err("simulation_rejected:") → send_bundle not reached
    #[tokio::test]
    async fn test_manager_sim_failure_blocks_bundle() {
        const SIM_FAIL: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"value":{"err":{"InstructionError":[0,"InvalidAccountData"]},"logs":[]}}}"#;
        const BUNDLE_OK: &str = r#"{"jsonrpc":"2.0","id":1,"result":"bundle_x"}"#;
        let bundle_called = Arc::new(AtomicBool::new(false));
        let jito = JitoClient::new(mock_bundle_endpoint(BUNDLE_OK, Arc::clone(&bundle_called)).await)
            .with_sim_rpc(mock_sim_rpc(SIM_FAIL).await);

        let result = sim_gate_then_bundle(&jito, dummy_tx(), true).await;

        assert!(result.unwrap_err().to_string().starts_with("simulation_rejected:"));
        assert!(!bundle_called.load(Ordering::SeqCst));
    }

    // JITO_SIMULATION_ENABLED=false → gate block skipped → send_bundle called
    #[tokio::test]
    async fn test_manager_sim_disabled_proceeds_to_bundle() {
        const BUNDLE_OK: &str = r#"{"jsonrpc":"2.0","id":1,"result":"bundle_y"}"#;
        let bundle_called = Arc::new(AtomicBool::new(false));
        // sim_rpc is unreachable; confirms gate is not contacted when disabled
        let jito = JitoClient::new(mock_bundle_endpoint(BUNDLE_OK, Arc::clone(&bundle_called)).await)
            .with_sim_rpc("http://127.0.0.1:1/unreachable".to_string());

        let result = sim_gate_then_bundle(&jito, dummy_tx(), false).await;

        assert_eq!(result.unwrap(), "bundle_y");
        assert!(bundle_called.load(Ordering::SeqCst));
    }

    // No sim RPC configured → gate returns Ok() immediately → send_bundle called
    #[tokio::test]
    async fn test_manager_no_sim_rpc_proceeds_to_bundle() {
        const BUNDLE_OK: &str = r#"{"jsonrpc":"2.0","id":1,"result":"bundle_z"}"#;
        let bundle_called = Arc::new(AtomicBool::new(false));
        let jito = JitoClient::new(mock_bundle_endpoint(BUNDLE_OK, Arc::clone(&bundle_called)).await);

        let result = sim_gate_then_bundle(&jito, dummy_tx(), true).await;

        assert_eq!(result.unwrap(), "bundle_z");
        assert!(bundle_called.load(Ordering::SeqCst));
    }
}
