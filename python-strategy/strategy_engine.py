import asyncio
import math
import os
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

PRESET_PARAMS: Dict[str, Dict] = {
    "conservative": {
        "buy_amount_sol": 0.05,
        "stop_loss_pct": 5.0,
        "take_profit_pct": 20.0,
        "max_positions": 2,
    },
    "balanced": {
        "buy_amount_sol": 0.15,
        "stop_loss_pct": 10.0,
        "take_profit_pct": 50.0,
        "max_positions": 5,
    },
    "aggressive": {
        "buy_amount_sol": 0.5,
        "stop_loss_pct": 20.0,
        "take_profit_pct": 100.0,
        "max_positions": 10,
    },
}

PRESET_REFRESH_CYCLES = 12


def _compute_advanced_metrics(pnl_series: List[float]) -> Dict:
    """
    Compute Sharpe ratio, max drawdown, and volatility from a list of per-trade PnL values (in SOL).
    Returns zeroed dict when insufficient data.
    """
    if not pnl_series or len(pnl_series) < 2:
        return {
            "sharpe_ratio": 0.0,
            "max_drawdown_sol": 0.0,
            "volatility_sol": 0.0,
        }

    n = len(pnl_series)
    mean = sum(pnl_series) / n
    variance = sum((x - mean) ** 2 for x in pnl_series) / (n - 1)
    std = math.sqrt(variance) if variance > 0 else 1e-9

    # Annualise assuming each trade ≈ 5 minutes (288 trades/day, 365 days)
    annualization = math.sqrt(288 * 365)
    sharpe = (mean / std) * annualization

    # Max drawdown: peak-to-trough on cumulative PnL curve
    cumulative = 0.0
    peak = 0.0
    max_dd = 0.0
    for p in pnl_series:
        cumulative += p
        if cumulative > peak:
            peak = cumulative
        dd = peak - cumulative
        if dd > max_dd:
            max_dd = dd

    return {
        "sharpe_ratio": round(sharpe, 4),
        "max_drawdown_sol": round(max_dd, 6),
        "volatility_sol": round(std, 6),
    }


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
    6. Reads strategy_preset from wallet_config and applies risk params to strategies
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
        self.data_collector = PumpFunDataCollector(self.grpc_client)
        self.data_collector.register_callback(self._on_token_event)

        self._orders_submitted_count = 0
        self._orders_failed_count = 0
        self._win_count = 0
        self._total_pnl_sol = 0.0

        self._active_preset: str = "balanced"
        self._scan_cycle: int = 0

        self._express_api = os.environ.get("EXPRESS_API_URL", "http://localhost:8080")
        self._wallet_id: str = "wallet_001"

    async def start(self):
        logger.info("Starting strategy engine")
        self.running = True

        _start_prometheus()

        await self.grpc_client.connect()

        # Load preset from wallet_config at startup so strategies use the
        # correct risk params from the very first evaluation cycle.
        await self._refresh_preset_from_wallet_config()

        self._scan_task = asyncio.create_task(self._market_scan_loop())
        await self.data_collector.start()

        logger.info("Strategy engine started", strategies=[s.name for s in self.strategies])

    async def stop(self):
        logger.info("Stopping strategy engine")
        self.running = False

        if self._scan_task:
            self._scan_task.cancel()

        await self.data_collector.stop()
        await self.grpc_client.disconnect()
        logger.info("Strategy engine stopped")

    def _apply_preset(self, preset: str) -> None:
        """Apply risk parameters from a named preset to all strategies immediately."""
        params = PRESET_PARAMS.get(preset)
        if params is None:
            logger.warning("Unknown preset, keeping current params", preset=preset)
            return

        changed = preset != self._active_preset
        self._active_preset = preset

        for strategy in self.strategies:
            if hasattr(strategy, "buy_amount_sol"):
                strategy.buy_amount_sol = params["buy_amount_sol"]
            if hasattr(strategy, "stop_loss_pct"):
                strategy.stop_loss_pct = params["stop_loss_pct"]
            if hasattr(strategy, "take_profit_pct"):
                strategy.take_profit_pct = params["take_profit_pct"]

        if changed:
            logger.info(
                "Strategy preset applied",
                preset=preset,
                buy_amount_sol=params["buy_amount_sol"],
                stop_loss_pct=params["stop_loss_pct"],
                take_profit_pct=params["take_profit_pct"],
            )

    async def _refresh_preset_from_wallet_config(self) -> None:
        """
        Fetch wallet_config.strategy_preset from the Express API and apply it.
        Called at startup and periodically — errors are logged but never raised.
        """
        try:
            import aiohttp
            url = f"{self._express_api}/api/wallets/{self._wallet_id}/config"
            async with aiohttp.ClientSession() as session:
                async with session.get(url, timeout=aiohttp.ClientTimeout(total=4)) as resp:
                    if resp.status == 200:
                        data = await resp.json()
                        preset = data.get("strategyPreset") or data.get("strategy_preset") or "balanced"
                        self._apply_preset(preset)
                    elif resp.status == 404:
                        logger.debug("wallet_config not found, using default preset", wallet_id=self._wallet_id)
                        self._apply_preset("balanced")
        except Exception as exc:
            logger.debug("Could not refresh preset from wallet_config", error=str(exc))

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
                self._scan_cycle += 1
                # Refresh preset from wallet_config every PRESET_REFRESH_CYCLES cycles
                # (avoids a DB/HTTP call on every 2-second scan tick).
                if self._scan_cycle % PRESET_REFRESH_CYCLES == 0:
                    await self._refresh_preset_from_wallet_config()
                await self._run_strategies()
            except asyncio.CancelledError:
                break
            except Exception as e:
                logger.error("Market scan error", error=str(e))
            await asyncio.sleep(settings.market_scan_interval_seconds)

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

        # Collect per-trade PnL history from strategies (for advanced metrics)
        pnl_series = []
        for s in self.strategies:
            if hasattr(s, "pnl_history") and s.pnl_history:
                pnl_series.extend(s.pnl_history)

        advanced = _compute_advanced_metrics(pnl_series)

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
            "circuit_breaker_state": self.grpc_client.circuit_state,
            "active_preset": self._active_preset,
            **advanced,
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
