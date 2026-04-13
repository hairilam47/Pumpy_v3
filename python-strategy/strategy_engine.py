import asyncio
import time
from typing import Dict, List, Optional
import structlog
from prometheus_client import Counter, Gauge, Histogram, start_http_server

from config import settings
from grpc_client import BotGrpcClient
from strategies import SniperStrategy, MomentumStrategy
from strategies.base import TokenMarketData, TradeSignal
from analytics.data_collector import PumpFunDataCollector

logger = structlog.get_logger(__name__)

signals_generated = Counter("strategy_signals_total", "Total signals generated", ["strategy", "direction"])
orders_submitted = Counter("strategy_orders_submitted_total", "Orders submitted", ["strategy"])
orders_failed = Counter("strategy_orders_failed_total", "Orders failed", ["strategy"])
active_tokens = Gauge("strategy_active_tokens", "Tokens being monitored")
signal_latency = Histogram("strategy_signal_latency_seconds", "Signal generation latency")

_prometheus_started = False


def _start_prometheus():
    global _prometheus_started
    if not _prometheus_started:
        try:
            start_http_server(9092)
            _prometheus_started = True
            logger.info("Prometheus metrics server started", port=9092)
        except Exception as exc:
            logger.warning("Could not start Prometheus server", error=str(exc))


class StrategyEngine:
    """
    Main strategy orchestrator:
    1. Monitors Pump.fun tokens via gRPC from the Rust engine
    2. Runs Sniper and Momentum strategies
    3. Generates ML-based signals
    4. Submits orders back to the Rust engine via gRPC
    5. Exposes Prometheus metrics on port 9092
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
        self._collector_task: Optional[asyncio.Task] = None
        self.data_collector = PumpFunDataCollector(self.grpc_client)
        self.data_collector.register_callback(self._on_token_event)

        self._orders_submitted_count = 0
        self._orders_failed_count = 0
        self._win_count = 0
        self._total_pnl_sol = 0.0

    async def start(self):
        logger.info("Starting strategy engine")
        self.running = True

        _start_prometheus()

        await self.grpc_client.connect()

        self._scan_task = asyncio.create_task(self._market_scan_loop())
        self._order_stream_task = asyncio.create_task(self._order_stream_loop())

        await self.data_collector.start()

        logger.info("Strategy engine started", strategies=[s.name for s in self.strategies])

    async def stop(self):
        logger.info("Stopping strategy engine")
        self.running = False

        if self._scan_task:
            self._scan_task.cancel()
        if self._order_stream_task:
            self._order_stream_task.cancel()

        await self.data_collector.stop()
        await self.grpc_client.disconnect()
        logger.info("Strategy engine stopped")

    async def _on_token_event(self, token: TokenMarketData):
        """Callback from the data collector — add new tokens to tracked set."""
        if token.mint not in self.tracked_tokens:
            self.tracked_tokens[token.mint] = token
            logger.debug("New token from stream", mint=token.mint[:12])
        else:
            existing = self.tracked_tokens[token.mint]
            if token.price > 0:
                existing.price = token.price
                if token.price not in existing.price_history:
                    existing.price_history.append(token.price)
                    existing.price_history = existing.price_history[-100:]

    async def _market_scan_loop(self):
        while self.running:
            try:
                await self._run_strategies()
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error("Market scan error", error=str(e))
            await asyncio.sleep(settings.market_scan_interval_seconds)

    async def _order_stream_loop(self):
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
        if not self.tracked_tokens:
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
            self._orders_submitted_count += 1
            logger.info(
                "Order submitted",
                order_id=result.get("order_id"),
                strategy=signal.strategy_name,
            )
        else:
            orders_failed.labels(strategy=signal.strategy_name).inc()
            self._orders_failed_count += 1
            logger.warning(
                "Order submission failed",
                reason=result.get("message"),
                strategy=signal.strategy_name,
            )

    def add_token(self, token: TokenMarketData):
        self.tracked_tokens[token.mint] = token

    def update_token(self, mint: str, **kwargs):
        if mint in self.tracked_tokens:
            token = self.tracked_tokens[mint]
            for key, value in kwargs.items():
                if hasattr(token, key):
                    setattr(token, key, value)

    def remove_token(self, mint: str):
        self.tracked_tokens.pop(mint, None)

    def get_strategy_stats(self) -> List[Dict]:
        return [s.get_stats() for s in self.strategies]

    def get_metrics(self) -> Dict:
        """Aggregate metrics across all strategies for the /metrics endpoint."""
        total_trades = sum(s.trades_executed for s in self.strategies)
        total_wins = sum(s.trades_won for s in self.strategies)
        total_pnl = sum(s.total_pnl for s in self.strategies)
        win_rate = (total_wins / total_trades * 100.0) if total_trades > 0 else 0.0

        return {
            "total_trades": total_trades,
            "total_wins": total_wins,
            "total_losses": total_trades - total_wins,
            "win_rate": win_rate,
            "total_pnl_sol": total_pnl,
            "open_positions": len(self.tracked_tokens),
            "orders_submitted": self._orders_submitted_count,
            "orders_failed": self._orders_failed_count,
            "strategies": self.get_strategy_stats(),
            "data_collector": self.data_collector.get_stats(),
            "grpc_connected": self.grpc_client.connected,
        }

    def _seed_mock_tokens(self):
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
