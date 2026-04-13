import asyncio
import grpc
import structlog
from typing import AsyncIterator, Optional, Dict
from tenacity import retry, stop_after_attempt, wait_exponential

from config import settings

logger = structlog.get_logger(__name__)


class BotGrpcClient:
    """gRPC client for communicating with the Rust trading engine."""

    def __init__(self):
        self.channel: Optional[grpc.aio.Channel] = None
        self.stub = None
        self.bot_pb2 = None
        self._connected = False
        self._reconnect_task: Optional[asyncio.Task] = None
        self._stopped = False

        # Load proto stubs at import time (not during connect)
        try:
            from grpc_client import bot_pb2, bot_pb2_grpc
        except ImportError:
            from . import bot_pb2, bot_pb2_grpc  # type: ignore[no-redef]

        self._bot_pb2 = bot_pb2
        self._bot_pb2_grpc = bot_pb2_grpc

    async def connect(self):
        """Open gRPC channel and attempt initial connection; start reconnect loop."""
        self._stopped = False
        target = f"{settings.grpc_host}:{settings.grpc_port}"
        logger.info("Connecting to Rust engine", target=target)

        self.channel = grpc.aio.insecure_channel(
            target,
            options=[
                ("grpc.keepalive_time_ms", 10_000),
                ("grpc.keepalive_timeout_ms", 5_000),
                ("grpc.keepalive_permit_without_calls", True),
                ("grpc.http2.max_pings_without_data", 0),
            ],
        )
        self.stub = self._bot_pb2_grpc.BotStub(self.channel)
        self.bot_pb2 = self._bot_pb2

        # Non-blocking: try initial probe; start reconnect loop regardless
        await self._probe_connection(target)
        self._reconnect_task = asyncio.create_task(self._reconnect_loop(target))

    async def _probe_connection(self, target: str):
        """Check whether the Rust engine is reachable right now."""
        try:
            await asyncio.wait_for(self.channel.channel_ready(), timeout=3.0)
            if not self._connected:
                self._connected = True
                logger.info("Connected to Rust engine", target=target)
        except (asyncio.TimeoutError, Exception):
            if self._connected:
                self._connected = False
                logger.warning("Lost connection to Rust engine", target=target)
            else:
                logger.info("Rust engine not reachable yet (standalone mode)", target=target)

    async def _reconnect_loop(self, target: str):
        """Background task: periodically re-probe until connected or stopped."""
        delay = 5.0
        while not self._stopped:
            await asyncio.sleep(delay)
            if self._stopped:
                break
            if not self._connected:
                await self._probe_connection(target)
                delay = min(delay * 1.5, 60.0)
            else:
                delay = 5.0

    async def disconnect(self):
        """Close the gRPC channel and stop the reconnect loop."""
        self._stopped = True
        if self._reconnect_task and not self._reconnect_task.done():
            self._reconnect_task.cancel()
            try:
                await self._reconnect_task
            except asyncio.CancelledError:
                pass
        if self.channel:
            await self.channel.close()
            self._connected = False
            logger.info("Disconnected from Rust engine")

    @property
    def connected(self) -> bool:
        return self._connected

    @retry(stop=stop_after_attempt(3), wait=wait_exponential(multiplier=1, min=1, max=10))
    async def submit_order(
        self,
        token_mint: str,
        side: str,
        amount: int,
        order_type: str = "MARKET",
        slippage_bps: int = 100,
        strategy_name: str = "python_strategy",
        metadata: Optional[Dict[str, str]] = None,
        price: Optional[float] = None,
        max_sol_cost: Optional[int] = None,
        min_sol_output: Optional[int] = None,
    ) -> Dict:
        """Submit an order to the Rust engine."""
        if not self._connected:
            logger.warning("Not connected to Rust engine, order not submitted",
                           token_mint=token_mint, side=side)
            return {"success": False, "message": "Not connected to Rust engine", "order_id": ""}

        try:
            request = self.bot_pb2.SubmitOrderRequest(
                token_mint=token_mint,
                side=side.upper(),
                amount=amount,
                order_type=order_type.upper(),
                slippage_bps=slippage_bps,
                strategy_name=strategy_name,
                metadata=metadata or {},
                price=price,
                max_sol_cost=max_sol_cost,
                min_sol_output=min_sol_output,
            )
            response = await self.stub.SubmitOrder(request, timeout=10.0)
            return {
                "success": response.success,
                "order_id": response.order_id,
                "message": response.message,
            }
        except grpc.RpcError as e:
            logger.error("gRPC SubmitOrder error", error=str(e))
            raise

    async def cancel_order(self, order_id: str) -> Dict:
        """Cancel an order."""
        if not self._connected:
            return {"success": False, "message": "Not connected"}
        try:
            request = self.bot_pb2.CancelOrderRequest(order_id=order_id)
            response = await self.stub.CancelOrder(request, timeout=5.0)
            return {"success": response.success, "message": response.message}
        except grpc.RpcError as e:
            logger.error("gRPC CancelOrder error", error=str(e))
            return {"success": False, "message": str(e)}

    async def get_order_status(self, order_id: str) -> Dict:
        """Get order status."""
        if not self._connected:
            return {"order_id": order_id, "status": "UNKNOWN"}
        try:
            request = self.bot_pb2.GetOrderStatusRequest(order_id=order_id)
            response = await self.stub.GetOrderStatus(request, timeout=5.0)
            return {
                "order_id": response.order_id,
                "status": response.status,
                "signature": response.signature,
                "error": response.error,
                "executed_at": response.executed_at,
            }
        except grpc.RpcError as e:
            logger.error("gRPC GetOrderStatus error", error=str(e))
            return {"order_id": order_id, "status": "ERROR", "error": str(e)}

    async def get_token_info(self, token_mint: str) -> Dict:
        """Get token information from the Rust engine."""
        if not self._connected:
            return {}
        try:
            request = self.bot_pb2.GetTokenInfoRequest(token_mint=token_mint)
            response = await self.stub.GetTokenInfo(request, timeout=5.0)
            return {
                "mint": response.mint,
                "name": response.name,
                "symbol": response.symbol,
                "price": response.price,
                "liquidity_sol": response.liquidity_sol,
                "market_cap_sol": response.market_cap_sol,
                "volume_24h_sol": response.volume_24h_sol,
                "holder_count": response.holder_count,
                "bonding_curve_progress": response.bonding_curve_progress,
            }
        except grpc.RpcError as e:
            logger.error("gRPC GetTokenInfo error", error=str(e))
            return {}

    async def get_portfolio_summary(self) -> Dict:
        """Get portfolio summary from the Rust engine."""
        if not self._connected:
            return self._mock_portfolio()
        try:
            request = self.bot_pb2.Empty()
            response = await self.stub.GetPortfolioSummary(request, timeout=5.0)
            return {
                "total_value_sol": response.total_value_sol,
                "cash_balance_sol": response.cash_balance_sol,
                "positions_value_sol": response.positions_value_sol,
                "daily_pnl_sol": response.daily_pnl_sol,
                "total_pnl_sol": response.total_pnl_sol,
                "open_positions_count": response.open_positions_count,
                "win_rate": response.win_rate,
            }
        except grpc.RpcError as e:
            logger.error("gRPC GetPortfolioSummary error", error=str(e))
            return self._mock_portfolio()

    async def stream_orders(self, order_ids=None) -> AsyncIterator[Dict]:
        """Stream order updates."""
        if not self._connected:
            return
        try:
            request = self.bot_pb2.StreamOrdersRequest(order_ids=order_ids or [])
            async for update in self.stub.StreamOrders(request):
                yield {
                    "order_id": update.order_id,
                    "token_mint": update.token_mint,
                    "status": update.status,
                    "signature": update.signature,
                    "error": update.error,
                    "executed_at": update.executed_at,
                    "executed_price": update.executed_price,
                    "executed_amount": update.executed_amount,
                }
        except grpc.RpcError as e:
            logger.error("gRPC StreamOrders error", error=str(e))

    def _mock_portfolio(self) -> Dict:
        """Return mock portfolio data when not connected."""
        return {
            "total_value_sol": 10.0,
            "cash_balance_sol": 9.5,
            "positions_value_sol": 0.5,
            "daily_pnl_sol": 0.02,
            "total_pnl_sol": 0.15,
            "open_positions_count": 1,
            "win_rate": 62.5,
        }
