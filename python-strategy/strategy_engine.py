import asyncio
import time
from typing import Dict, List, Optional
import structlog
from prometheus_client import Counter, Gauge, Histogram, start_http_server

from config import settings
from grpc_client import BotGrpcClient
from strategies import SniperStrategy, MomentumStrategy
from strategies.base import TokenMarketData, TradeSignal

logger = structlog.get_logger(__name__)

# Prometheus metrics
signals_generated = Counter("strategy_signals_total", "Total signals generated", ["strategy", "direction"])
orders_submitted = Counter("strategy_orders_submitted_total", "Orders submitted", ["strategy"])
orders_failed = Counter("strategy_orders_failed_total", "Orders failed", ["strategy"])
active_tokens = Gauge("strategy_active_tokens", "Tokens being monitored")
signal_latency = Histogram("strategy_signal_latency_seconds", "Signal generation latency")


class StrategyEngine:
    """
    Main strategy orchestrator that:
    1. Monitors Pump.fun tokens via gRPC from the Rust engine
    2. Runs Sniper and Momentum strategies
    3. Generates ML-based signals
    4. Submits orders back to the Rust engine via gRPC
    """

    def __init__(self):
        self.grpc_client = BotGrpcClient()
        self.strategies = [
            SniperStrategy(),
            MomentumStrategy(),
        ]
        self.tracked_tokens: Dict[str, TokenMarketData] = {}
        self.running = False
        self._scan_task: Optional[asyncio.Task] = None
        self._order_stream_task: Optional[asyncio.Task] = None

    async def start(self):
        """Start the strategy engine."""
        logger.info("Starting strategy engine")
        self.running = True

        # Connect to Rust engine
        await self.grpc_client.connect()

        # Start background tasks
        self._scan_task = asyncio.create_task(self._market_scan_loop())
        self._order_stream_task = asyncio.create_task(self._order_stream_loop())

        logger.info("Strategy engine started", strategies=[s.name for s in self.strategies])

    async def stop(self):
        """Stop the strategy engine gracefully."""
        logger.info("Stopping strategy engine")
        self.running = False

        if self._scan_task:
            self._scan_task.cancel()
        if self._order_stream_task:
            self._order_stream_task.cancel()

        await self.grpc_client.disconnect()
        logger.info("Strategy engine stopped")

    async def _market_scan_loop(self):
        """Periodically scan market and evaluate strategies."""
        while self.running:
            try:
                await self._run_strategies()
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error("Market scan error", error=str(e))
            await asyncio.sleep(settings.market_scan_interval_seconds)

    async def _order_stream_loop(self):
        """Listen to order updates from the Rust engine."""
        while self.running:
            try:
                async for update in self.grpc_client.stream_orders():
                    logger.info(
                        "Order update",
                        order_id=update.get("order_id"),
                        status=update.get("status"),
                        signature=update.get("signature"),
                    )
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.warning("Order stream error, retrying", error=str(e))
                await asyncio.sleep(5)

    async def _run_strategies(self):
        """Run all enabled strategies against tracked tokens."""
        if not self.tracked_tokens:
            # Seed with some mock tokens for demo purposes when not connected
            if not self.grpc_client.connected:
                self._seed_mock_tokens()
            else:
                return

        active_tokens.set(len(self.tracked_tokens))

        for mint, token in list(self.tracked_tokens.items()):
            for strategy in self.strategies:
                if not strategy.enabled:
                    continue
                try:
                    start = time.monotonic()
                    signal = await strategy.analyze(token)
                    latency = time.monotonic() - start
                    signal_latency.observe(latency)

                    if signal:
                        signals_generated.labels(
                            strategy=strategy.name,
                            direction=signal.side,
                        ).inc()
                        await self._execute_signal(signal)
                except Exception as e:
                    logger.error("Strategy error", strategy=strategy.name, error=str(e))

    async def _execute_signal(self, signal: TradeSignal):
        """Execute a trade signal by submitting to the Rust engine."""
        logger.info(
            "Executing signal",
            strategy=signal.strategy_name,
            side=signal.side,
            token=signal.token_mint[:8] + "...",
            amount_sol=signal.amount_sol,
            confidence=signal.confidence,
            reason=signal.reason,
        )

        amount_lamports = int(signal.amount_sol * 1_000_000_000)

        result = await self.grpc_client.submit_order(
            token_mint=signal.token_mint,
            side=signal.side,
            amount=amount_lamports,
            order_type="MARKET",
            slippage_bps=signal.slippage_bps,
            strategy_name=signal.strategy_name,
            metadata=signal.metadata,
        )

        if result.get("success"):
            orders_submitted.labels(strategy=signal.strategy_name).inc()
            logger.info(
                "Order submitted",
                order_id=result.get("order_id"),
                strategy=signal.strategy_name,
            )
        else:
            orders_failed.labels(strategy=signal.strategy_name).inc()
            logger.warning(
                "Order submission failed",
                reason=result.get("message"),
                strategy=signal.strategy_name,
            )

    def add_token(self, token: TokenMarketData):
        """Add a token to the monitored set."""
        self.tracked_tokens[token.mint] = token

    def update_token(self, mint: str, **kwargs):
        """Update a tracked token's market data."""
        if mint in self.tracked_tokens:
            token = self.tracked_tokens[mint]
            for key, value in kwargs.items():
                if hasattr(token, key):
                    setattr(token, key, value)

    def remove_token(self, mint: str):
        """Remove a token from monitoring."""
        self.tracked_tokens.pop(mint, None)

    def get_strategy_stats(self) -> List[Dict]:
        """Get performance stats for all strategies."""
        return [s.get_stats() for s in self.strategies]

    def _seed_mock_tokens(self):
        """Seed with mock token data for demo/development."""
        import random
        from datetime import datetime

        mints = [
            "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU",
            "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263",
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        ]
        for mint in mints:
            if mint not in self.tracked_tokens:
                price = random.uniform(0.000001, 0.001)
                self.tracked_tokens[mint] = TokenMarketData(
                    mint=mint,
                    name=f"Token {mint[:4]}",
                    symbol=f"TKN{mint[:4]}",
                    price=price,
                    liquidity_sol=random.uniform(2.0, 50.0),
                    market_cap_sol=random.uniform(10.0, 300.0),
                    volume_24h_sol=random.uniform(1.0, 100.0),
                    holder_count=random.randint(10, 500),
                    bonding_curve_progress=random.uniform(5.0, 70.0),
                    price_history=[price * (1 + random.gauss(0, 0.02)) for _ in range(30)],
                    volume_history=[random.uniform(0.5, 20.0) for _ in range(30)],
                    created_at=datetime.utcnow(),
                )
