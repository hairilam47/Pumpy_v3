use std::sync::Arc;
use std::str::FromStr;
use std::pin::Pin;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};
use tracing::{info, error};

use crate::metrics::Metrics;
use crate::order::{Order, OrderManager, OrderSide, OrderStatus, OrderType};
use crate::pumpfun::PumpFunClient;

// Include generated protobuf code
pub mod bot_proto {
    tonic::include_proto!("bot");
}

use bot_proto::bot_server::Bot;
use bot_proto::*;

#[derive(Clone)]
pub struct BotService {
    order_manager: Arc<OrderManager>,
    pumpfun_client: Arc<PumpFunClient>,
    metrics: Arc<Metrics>,
    /// When true the engine started without a real wallet (ephemeral keypair).
    /// All trade-execution RPCs are rejected in this mode.
    demo_mode: bool,
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
        info!("SubmitOrder: {} {} {}", req.side, req.token_mint, req.amount);

        let order_type = OrderType::from_str(&req.order_type).map_err(|e| {
            Status::invalid_argument(format!("Invalid order type: {}", e))
        })?;

        let side = OrderSide::from_str(&req.side).map_err(|e| {
            Status::invalid_argument(format!("Invalid order side: {}", e))
        })?;

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
        };

        match self.order_manager.submit_order(order).await {
            Ok(order_id) => Ok(Response::new(SubmitOrderResponse {
                order_id,
                success: true,
                message: "Order submitted successfully".to_string(),
            })),
            Err(e) => {
                error!("Failed to submit order: {}", e);
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
