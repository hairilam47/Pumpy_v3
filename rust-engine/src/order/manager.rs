use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc, broadcast, Semaphore};
use uuid::Uuid;
use chrono::Utc;
use tracing::{info, warn, error};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

use crate::database::DatabasePool;
use crate::metrics::Metrics;
use crate::mev::MevProtector;
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
        }
    }
}

pub struct OrderManager {
    db_pool: DatabasePool,
    pumpfun_client: Arc<PumpFunClient>,
    mev_protector: Arc<MevProtector>,
    metrics: Arc<Metrics>,
    pending_orders: Arc<RwLock<VecDeque<Order>>>,
    active_orders: Arc<RwLock<HashMap<String, Order>>>,
    order_history: Arc<RwLock<HashMap<String, Order>>>,
    order_tx: mpsc::UnboundedSender<Order>,
    order_rx: Arc<RwLock<mpsc::UnboundedReceiver<Order>>>,
    event_tx: broadcast::Sender<OrderEvent>,
    config: OrderManagerConfig,
}

#[derive(Debug, Clone)]
pub struct OrderEvent {
    pub order_id: String,
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
        metrics: Arc<Metrics>,
        config: OrderManagerConfig,
    ) -> Self {
        let (order_tx, order_rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(1000);

        Self {
            db_pool,
            pumpfun_client,
            mev_protector,
            metrics,
            pending_orders: Arc::new(RwLock::new(VecDeque::new())),
            active_orders: Arc::new(RwLock::new(HashMap::new())),
            order_history: Arc::new(RwLock::new(HashMap::new())),
            order_tx,
            order_rx: Arc::new(RwLock::new(order_rx)),
            event_tx,
            config,
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<OrderEvent> {
        self.event_tx.subscribe()
    }

    pub fn db_pool(&self) -> &DatabasePool {
        &self.db_pool
    }

    /// Submit a new order for execution
    pub async fn submit_order(&self, mut order: Order) -> Result<String, OrderError> {
        self.validate_order(&order)?;
        self.risk_check(&order).await?;

        order.id = Uuid::new_v4().to_string();
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

        // MEV sandwich risk check
        let accounts = vec![order.mint.clone()];
        let risk = self.mev_protector.analyze_sandwich_risk(&order.mint, &accounts).await;
        if risk.score > self.config.max_sandwich_risk_score {
            order.status = OrderStatus::Failed;
            order.error = Some(format!("Sandwich risk too high: score={}", risk.score));
            order.updated_at = Utc::now();
            self.finalize_order(order).await?;
            self.metrics.orders_rejected.inc();
            return Err(OrderError::SandwichRiskTooHigh(risk.score));
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

        let result = match order.side {
            OrderSide::Buy => {
                let max_cost = order.max_cost.unwrap_or(order.amount + order.amount * order.slippage_bps / 10_000);
                self.pumpfun_client
                    .buy_token(&mint, order.amount, max_cost, order.slippage_bps)
                    .await
            }
            OrderSide::Sell => {
                let min_output = order.min_output.unwrap_or(
                    order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000),
                );
                self.pumpfun_client
                    .sell_token(&mint, order.amount, min_output, order.slippage_bps)
                    .await
            }
        };

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
            status: order.status.to_string(),
            signature: order.signature.clone(),
            error: order.error.clone(),
            executed_at: order.executed_at.map(|t| t.to_rfc3339()),
            executed_price: order.executed_price,
            executed_amount: order.executed_amount,
        };
        let _ = self.event_tx.send(event);
    }

    fn validate_order(&self, order: &Order) -> Result<(), OrderError> {
        if order.amount == 0 {
            return Err(OrderError::InvalidAmount);
        }
        if order.slippage_bps > 1000 {
            return Err(OrderError::SlippageTooHigh);
        }
        Ok(())
    }

    async fn risk_check(&self, order: &Order) -> Result<(), OrderError> {
        let trade_value_sol = order.amount as f64 / 1_000_000_000.0;
        if trade_value_sol > self.config.max_position_size_sol {
            return Err(OrderError::PositionSizeTooLarge);
        }
        Ok(())
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
            metrics: self.metrics.clone(),
            active_orders: self.active_orders.clone(),
            order_history: self.order_history.clone(),
            order_tx: self.order_tx.clone(),
            event_tx: self.event_tx.clone(),
            config: self.config.clone(),
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
    metrics: Arc<Metrics>,
    active_orders: Arc<RwLock<HashMap<String, Order>>>,
    order_history: Arc<RwLock<HashMap<String, Order>>>,
    order_tx: mpsc::UnboundedSender<Order>,
    event_tx: broadcast::Sender<OrderEvent>,
    config: OrderManagerConfig,
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
        if risk.score > self.config.max_sandwich_risk_score {
            order.status = OrderStatus::Failed;
            order.error = Some(format!("Sandwich risk too high: score={}", risk.score));
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

        let result = match order.side {
            OrderSide::Buy => {
                let max_cost = order.max_cost.unwrap_or(order.amount + order.amount * order.slippage_bps / 10_000);
                self.pumpfun_client.buy_token(&mint, order.amount, max_cost, order.slippage_bps).await
            }
            OrderSide::Sell => {
                let min_output = order.min_output.unwrap_or(
                    order.amount.saturating_sub(order.amount * order.slippage_bps / 10_000),
                );
                self.pumpfun_client.sell_token(&mint, order.amount, min_output, order.slippage_bps).await
            }
        };

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
