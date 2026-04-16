use std::collections::HashMap;
use std::sync::Arc;
use std::str::FromStr;
use std::pin::Pin;
use std::time::{Instant, Duration};
use tokio::sync::RwLock;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::{info, warn, error};

use crate::database::{self, DbPool};
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
    /// Database pool for durable idempotency key persistence (Task #41).
    /// Used as crash-safe fallback when the in-memory cache misses.
    db_pool: DbPool,
}

impl BotService {
    pub fn new(
        order_manager: Arc<OrderManager>,
        pumpfun_client: Arc<PumpFunClient>,
        metrics: Arc<Metrics>,
        demo_mode: bool,
        db_pool: DbPool,
    ) -> Self {
        Self {
            order_manager,
            pumpfun_client,
            metrics,
            demo_mode,
            ikey_cache: make_ikey_cache(),
            db_pool,
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

        // ── Two-tier idempotency (Task #26 + Task #41) ──────────────────────────
        //
        // Tier 1 — in-memory cache (fast path, same-process dedup):
        //   Atomic check-and-reserve under a single write lock prevents TOCTOU for
        //   concurrent duplicates within the same process.
        //
        // Tier 2 — DB fallback (crash-safe, cross-restart dedup):
        //   When the in-memory cache misses (e.g. after a restart), we check the DB
        //   before reserving.  The DB INSERT is atomic (ON CONFLICT DO NOTHING) so
        //   two racing processes after a restart are handled safely.
        if !ikey.is_empty() {
            // --- Tier 1: in-memory check ---
            {
                let mut cache = self.ikey_cache.write().await;
                cache.retain(|_, v| v.recorded_at.elapsed() < Duration::from_secs(IKEY_TTL_SECS));

                match cache.get(&ikey) {
                    Some(entry) if !entry.order_id.is_empty() => {
                        warn!(
                            idempotency_key = %ikey,
                            existing_order_id = %entry.order_id,
                            trace_id = %trace_id,
                            "Duplicate request detected (memory) — returning existing order_id"
                        );
                        return Ok(Response::new(SubmitOrderResponse {
                            order_id: entry.order_id.clone(),
                            success: true,
                            message: "Duplicate: returning existing order".to_string(),
                        }));
                    }
                    Some(_) => {
                        warn!(
                            idempotency_key = %ikey,
                            trace_id = %trace_id,
                            "Concurrent duplicate in-flight (memory) — acknowledging"
                        );
                        return Ok(Response::new(SubmitOrderResponse {
                            order_id: String::new(),
                            success: true,
                            message: "Duplicate request acknowledged — order is being processed".to_string(),
                        }));
                    }
                    None => {
                        // Not in memory — insert a placeholder so concurrent same-process
                        // duplicates are blocked.  We will do the DB check below *outside*
                        // the write lock to avoid holding it during I/O.
                        cache.insert(ikey.clone(), IdempotencyEntry {
                            order_id: String::new(),
                            recorded_at: Instant::now(),
                        });
                    }
                }
            } // write lock released

            // --- Tier 2: DB fallback (crash-safe cross-restart dedup) ---
            //
            // The in-memory cache missed — this key either arrived fresh or was lost
            // in a crash.  Check the DB first, then atomically reserve if absent.
            //
            // IMPORTANT: DB errors must propagate as gRPC errors (Status::unavailable),
            // NOT silently treated as "key not found" or "reservation lost".  Swallowing
            // DB errors would drop valid orders while returning success to the caller.
            match database::check_idempotency_key(&self.db_pool, &ikey).await {
                Err(db_err) => {
                    // Transient DB failure — clean up the in-memory placeholder and let
                    // the caller retry.  Do NOT return success: that would silently drop
                    // the order while telling the caller it succeeded.
                    self.ikey_cache.write().await.remove(&ikey);
                    error!(
                        idempotency_key = %ikey,
                        trace_id = %trace_id,
                        error = %db_err,
                        "DB error checking idempotency key — rejecting request so caller can retry"
                    );
                    return Err(Status::unavailable(
                        "Idempotency DB temporarily unavailable — please retry",
                    ));
                }
                Ok(Some(row)) if !row.order_id.is_empty() => {
                    // Previously committed order found in DB — update memory cache and return.
                    {
                        let mut cache = self.ikey_cache.write().await;
                        if let Some(entry) = cache.get_mut(&ikey) {
                            entry.order_id = row.order_id.clone();
                        }
                    }
                    warn!(
                        idempotency_key = %ikey,
                        existing_order_id = %row.order_id,
                        trace_id = %trace_id,
                        "Duplicate request detected (DB crash-recovery) — returning existing order_id"
                    );
                    return Ok(Response::new(SubmitOrderResponse {
                        order_id: row.order_id,
                        success: true,
                        message: "Duplicate: returning existing order (crash recovery)".to_string(),
                    }));
                }
                Ok(Some(_)) => {
                    // Key exists in DB but has no order_id — previous run crashed mid-flight.
                    // Treat as in-flight to be safe (caller should retry after TTL).
                    warn!(
                        idempotency_key = %ikey,
                        trace_id = %trace_id,
                        "In-flight key found in DB after restart — acknowledging without re-execution"
                    );
                    return Ok(Response::new(SubmitOrderResponse {
                        order_id: String::new(),
                        success: true,
                        message: "Duplicate request acknowledged — order is being processed".to_string(),
                    }));
                }
                Ok(None) => {
                    // Key is absent from DB — atomically reserve it.
                    // ON CONFLICT DO NOTHING handles the rare race between two restarted
                    // processes that both missed the in-memory check simultaneously.
                    match database::reserve_idempotency_key(&self.db_pool, &ikey).await {
                        Err(db_err) => {
                            // Transient DB failure during reservation — reject with a retryable error.
                            self.ikey_cache.write().await.remove(&ikey);
                            error!(
                                idempotency_key = %ikey,
                                trace_id = %trace_id,
                                error = %db_err,
                                "DB error reserving idempotency key — rejecting request so caller can retry"
                            );
                            return Err(Status::unavailable(
                                "Idempotency DB temporarily unavailable — please retry",
                            ));
                        }
                        Ok(false) => {
                            // Another process beat us to the DB reservation (true duplicate race).
                            warn!(
                                idempotency_key = %ikey,
                                trace_id = %trace_id,
                                "Lost DB reservation race — acknowledging without re-execution"
                            );
                            self.ikey_cache.write().await.remove(&ikey);
                            return Ok(Response::new(SubmitOrderResponse {
                                order_id: String::new(),
                                success: true,
                                message: "Duplicate request acknowledged — order is being processed".to_string(),
                            }));
                        }
                        Ok(true) => {
                            // Won the DB reservation — proceed with order execution below.
                        }
                    }
                }
            }
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
            trace_id: Some(trace_id.clone()),
        };

        match self.order_manager.submit_order(order).await {
            Ok(order_id) => {
                // Commit the reservation in both memory and DB.
                if !ikey.is_empty() {
                    {
                        let mut cache = self.ikey_cache.write().await;
                        if let Some(entry) = cache.get_mut(&ikey) {
                            entry.order_id = order_id.clone();
                        }
                    }
                    database::commit_idempotency_key(&self.db_pool, &ikey, &order_id).await;
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
                    {
                        let mut cache = self.ikey_cache.write().await;
                        cache.remove(&ikey);
                    }
                    database::release_idempotency_key(&self.db_pool, &ikey).await;
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
