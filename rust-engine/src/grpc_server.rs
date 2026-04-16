use std::collections::HashMap;
use std::sync::Arc;
use std::str::FromStr;
use std::pin::Pin;
use std::time::{Instant, Duration};
use tokio::sync::RwLock;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::{info, warn, error};

use crate::metrics::Metrics;
use crate::order::{Order, OrderManager, OrderSide, OrderStatus, OrderType};
use crate::pumpfun::PumpFunClient;

// Include generated protobuf code
pub mod bot_proto {
    tonic::include_proto!("bot");
}

use bot_proto::bot_server::Bot;
use bot_proto::*;

// ── Idempotency cache (Task #26) ──────────────────────────────────────────────
const IKEY_TTL_SECS: u64 = 60; // 60-second dedup window

struct IdempotencyEntry {
    order_id: String,
    recorded_at: Instant,
}

type IdempotencyCache = Arc<RwLock<HashMap<String, IdempotencyEntry>>>;

fn make_ikey_cache() -> IdempotencyCache {
    Arc::new(RwLock::new(HashMap::new()))
}

// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct BotService {
    order_manager: Arc<OrderManager>,
    pumpfun_client: Arc<PumpFunClient>,
    metrics: Arc<Metrics>,
    /// When true the engine started without a real wallet (ephemeral keypair).
    /// All trade-execution RPCs are rejected in this mode.
    demo_mode: bool,
    ikey_cache: IdempotencyCache,
}

impl BotService {
    pub fn new(
        order_manager: Arc<OrderManager>,
        pumpfun_client: Arc<PumpFunClient>,
        metrics: Arc<Metrics>,
        demo_mode: bool,
    ) -> Self {
        Self {
            order_manager,
            pumpfun_client,
            metrics,
            demo_mode,
            ikey_cache: make_ikey_cache(),
        }
    }
}

#[tonic::async_trait]
impl Bot for BotService {
    async fn submit_order(
        &self,
        request: Request<SubmitOrderRequest>,
    ) -> Result<Response<SubmitOrderResponse>, Status> {
        if self.demo_mode {
            return Err(Status::failed_precondition(
                "Trading disabled: set WALLET_PRIVATE_KEY in Replit Secrets to enable live trading",
            ));
        }
        let req = request.into_inner();

        // Distributed tracing (Task #31)
        let trace_id = if req.trace_id.is_empty() { "no-trace".to_string() } else { req.trace_id.clone() };
        let ikey = req.idempotency_key.clone();

        // Parse and validate the client_order_id UUID before doing anything else.
        // Returns invalid_argument immediately if the caller sent a malformed UUID.
        let client_order_id: Option<uuid::Uuid> = if req.client_order_id.is_empty() {
            None
        } else {
            Some(uuid::Uuid::parse_str(&req.client_order_id).map_err(|_| {
                Status::invalid_argument(format!(
                    "client_order_id is not a valid UUID: {}",
                    req.client_order_id
                ))
            })?)
        };

        info!(
            trace_id = %trace_id,
            client_order_id = ?client_order_id,
            side = %req.side,
            mint = %req.token_mint,
            amount = req.amount,
            "SubmitOrder received"
        );

        // Validate request fields BEFORE touching the idempotency cache.
        // This prevents cache poisoning where an invalid request reserves a key and
        // blocks valid retries for the TTL window.
        let order_type = OrderType::from_str(&req.order_type).map_err(|e| {
            Status::invalid_argument(format!("Invalid order type: {}", e))
        })?;

        let side = OrderSide::from_str(&req.side).map_err(|e| {
            Status::invalid_argument(format!("Invalid order side: {}", e))
        })?;

        // Idempotency check (Task #26) — atomic check+reserve under single write lock.
        // Using a single write lock for both the check and the reservation prevents
        // the TOCTOU race where two concurrent retries both pass the read-check before
        // either inserts, which would cause duplicate execution.
        if !ikey.is_empty() {
            let mut cache = self.ikey_cache.write().await;
            // Evict stale entries while we hold the write lock.
            cache.retain(|_, v| v.recorded_at.elapsed() < Duration::from_secs(IKEY_TTL_SECS));

            match cache.get(&ikey) {
                Some(entry) if !entry.order_id.is_empty() => {
                    // Key seen before and order completed — return cached success response.
                    warn!(
                        idempotency_key = %ikey,
                        existing_order_id = %entry.order_id,
                        trace_id = %trace_id,
                        "Duplicate request detected — returning existing order_id"
                    );
                    return Ok(Response::new(SubmitOrderResponse {
                        order_id: entry.order_id.clone(),
                        success: true,
                        message: "Duplicate: returning existing order".to_string(),
                    }));
                }
                Some(_) => {
                    // Key is reserved but order is still in flight — concurrent duplicate.
                    // Return success acknowledgement; the caller should treat this as if the
                    // order was accepted (it was — the first copy is executing).
                    warn!(
                        idempotency_key = %ikey,
                        trace_id = %trace_id,
                        "Concurrent duplicate request in flight — acknowledging without re-execution"
                    );
                    return Ok(Response::new(SubmitOrderResponse {
                        order_id: String::new(),
                        success: true,
                        message: "Duplicate request acknowledged — order is being processed".to_string(),
                    }));
                }
                None => {
                    // First time we see this key: reserve it with an empty placeholder.
                    // Any concurrent duplicate will hit the Some(_) branch above.
                    cache.insert(ikey.clone(), IdempotencyEntry {
                        order_id: String::new(), // in-flight marker
                        recorded_at: Instant::now(),
                    });
                }
            }
            // Write lock is dropped here before the (potentially slow) order submission.
        }

        let order = Order {
            id: String::new(),
            mint: req.token_mint,
            order_type,
            side,
            amount: req.amount,
            price: req.price,
            max_cost: req.max_sol_cost,
            min_output: req.min_sol_output,
            slippage_bps: req.slippage_bps,
            status: OrderStatus::Pending,
            strategy: req.strategy_name,
            metadata: req.metadata,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            executed_at: None,
            signature: None,
            error: None,
            retry_count: 0,
            executed_price: None,
            executed_amount: None,
            client_order_id,
        };

        match self.order_manager.submit_order(order).await {
            Ok(order_id) => {
                // Commit the reservation: update in-flight marker with real order_id.
                if !ikey.is_empty() {
                    let mut cache = self.ikey_cache.write().await;
                    if let Some(entry) = cache.get_mut(&ikey) {
                        entry.order_id = order_id.clone();
                    }
                }
                info!(
                    trace_id = %trace_id,
                    order_id = %order_id,
                    "Order submitted successfully"
                );
                Ok(Response::new(SubmitOrderResponse {
                    order_id,
                    success: true,
                    message: "Order submitted successfully".to_string(),
                }))
            }
            Err(e) => {
                // Release the reservation so retries can go through.
                if !ikey.is_empty() {
                    let mut cache = self.ikey_cache.write().await;
                    cache.remove(&ikey);
                }
                error!(trace_id = %trace_id, error = %e, "Failed to submit order");
                Ok(Response::new(SubmitOrderResponse {
                    order_id: String::new(),
                    success: false,
                    message: e.to_string(),
                }))
            }
        }
    }

    async fn cancel_order(
        &self,
        request: Request<CancelOrderRequest>,
    ) -> Result<Response<CancelOrderResponse>, Status> {
        if self.demo_mode {
            return Err(Status::failed_precondition(
                "Trading disabled: set WALLET_PRIVATE_KEY in Replit Secrets to enable live trading",
            ));
        }
        let order_id = request.into_inner().order_id;
        info!("CancelOrder: {}", order_id);

        match self.order_manager.cancel_order(&order_id).await {
            Ok(_) => Ok(Response::new(CancelOrderResponse {
                success: true,
                message: "Order cancelled".to_string(),
            })),
            Err(e) => Ok(Response::new(CancelOrderResponse {
                success: false,
                message: e.to_string(),
            })),
        }
    }

    async fn get_order_status(
        &self,
        request: Request<GetOrderStatusRequest>,
    ) -> Result<Response<OrderStatusResponse>, Status> {
        let order_id = request.into_inner().order_id;

        match self.order_manager.get_order(&order_id).await {
            Some(order) => Ok(Response::new(OrderStatusResponse {
                order_id: order.id,
                status: order.status.to_string(),
                signature: order.signature.unwrap_or_default(),
                error: order.error.unwrap_or_default(),
                executed_at: order.executed_at.map(|t| t.to_rfc3339()),
            })),
            None => Err(Status::not_found(format!("Order not found: {}", order_id))),
        }
    }

    async fn get_token_info(
        &self,
        request: Request<GetTokenInfoRequest>,
    ) -> Result<Response<TokenInfoResponse>, Status> {
        let mint_str = request.into_inner().token_mint;
        let mint = solana_sdk::pubkey::Pubkey::from_str(&mint_str)
            .map_err(|e| Status::invalid_argument(format!("Invalid mint: {}", e)))?;

        let params = self.pumpfun_client
            .get_bonding_curve_params(&mint)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(TokenInfoResponse {
            mint: mint_str,
            name: String::new(),
            symbol: String::new(),
            price: params.token_price_lamports() / 1_000_000_000.0,
            liquidity_sol: params.real_sol_reserves as f64 / 1_000_000_000.0,
            market_cap_sol: params.market_cap_sol(),
            volume_24h_sol: 0.0,
            holder_count: 0,
            bonding_curve_progress: params.bonding_curve_progress(),
        }))
    }

    async fn get_portfolio_summary(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<PortfolioSummaryResponse>, Status> {
        let summary = self.order_manager.get_portfolio_summary().await;

        Ok(Response::new(PortfolioSummaryResponse {
            total_value_sol: summary.total_value_sol,
            cash_balance_sol: summary.cash_balance_sol,
            positions_value_sol: summary.positions_value_sol,
            daily_pnl_sol: summary.daily_pnl_sol,
            total_pnl_sol: summary.total_pnl_sol,
            open_positions_count: summary.open_positions_count,
            win_rate: summary.win_rate,
        }))
    }

    type StreamOrdersStream = Pin<Box<dyn Stream<Item = Result<OrderUpdate, Status>> + Send + 'static>>;

    async fn stream_orders(
        &self,
        request: Request<StreamOrdersRequest>,
    ) -> Result<Response<Self::StreamOrdersStream>, Status> {
        let filter_ids: std::collections::HashSet<String> = request
            .into_inner()
            .order_ids
            .into_iter()
            .collect();

        let mut rx = self.order_manager.subscribe_events();

        let stream = async_stream::try_stream! {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if filter_ids.is_empty() || filter_ids.contains(&event.order_id) {
                            yield OrderUpdate {
                                order_id: event.order_id,
                                token_mint: event.token_mint,
                                status: event.status,
                                signature: event.signature,
                                error: event.error,
                                executed_at: event.executed_at,
                                executed_price: event.executed_price,
                                executed_amount: event.executed_amount,
                            };
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }
}
