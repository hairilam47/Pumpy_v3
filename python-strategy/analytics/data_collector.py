"""
PumpFunDataCollector — subscribes to the Rust engine's StreamOrders gRPC stream
and feeds token events into active strategies.
"""

import asyncio
from datetime import datetime
from typing import Callable, Awaitable, List, Optional
import structlog

from strategies.base import TokenMarketData

logger = structlog.get_logger(__name__)

TokenEventCallback = Callable[[TokenMarketData], Awaitable[None]]


class PumpFunDataCollector:
    """
    Streams new-token events from the Rust engine's StreamOrders gRPC stream
    and distributes them to registered callbacks (i.e. active strategies).

    Usage:
        collector = PumpFunDataCollector(grpc_client)
        collector.register_callback(my_async_handler)
        await collector.start()
        ...
        await collector.stop()
    """

    def __init__(self, grpc_client):
        self.grpc_client = grpc_client
        self._callbacks: List[TokenEventCallback] = []
        self._task: Optional[asyncio.Task] = None
        self._running = False
        self.events_received = 0
        self.events_dispatched = 0

    def register_callback(self, callback: TokenEventCallback):
        """Register an async callback that receives TokenMarketData on each event."""
        self._callbacks.append(callback)

    def unregister_callback(self, callback: TokenEventCallback):
        """Remove a previously registered callback."""
        self._callbacks = [cb for cb in self._callbacks if cb is not callback]

    async def start(self):
        """Start streaming token events in the background."""
        if self._running:
            return
        self._running = True
        self._task = asyncio.create_task(self._stream_loop())
        logger.info("PumpFunDataCollector started", callbacks=len(self._callbacks))

    async def stop(self):
        """Stop the streaming loop."""
        self._running = False
        if self._task and not self._task.done():
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
        logger.info(
            "PumpFunDataCollector stopped",
            events_received=self.events_received,
            events_dispatched=self.events_dispatched,
        )

    async def _stream_loop(self):
        """Inner loop: subscribe to StreamOrders, parse events, fire callbacks."""
        while self._running:
            try:
                async for update in self.grpc_client.stream_orders():
                    if not self._running:
                        break
                    self.events_received += 1
                    token = self._parse_update(update)
                    if token is not None:
                        await self._dispatch(token)
            except asyncio.CancelledError:
                break
            except Exception as exc:
                logger.warning(
                    "DataCollector stream error — retrying in 5 s",
                    error=str(exc),
                )
                await asyncio.sleep(5)

    def _parse_update(self, update: dict) -> Optional[TokenMarketData]:
        """
        Convert a raw StreamOrders update dict into a TokenMarketData object.

        The Rust engine emits order-lifecycle events; we surface new-token
        events (status == "NEW" or "DETECTED") as TokenMarketData so
        strategies can evaluate them.
        """
        status = update.get("status", "")
        token_mint = update.get("token_mint") or update.get("order_id", "")

        if not token_mint or len(token_mint) < 32:
            return None

        try:
            price = float(update.get("executed_price", 0.0) or 0.0)
            amount = int(update.get("executed_amount", 0) or 0)
            liquidity_sol = amount / 1_000_000_000 if amount else 0.0

            return TokenMarketData(
                mint=token_mint,
                name=update.get("name", f"Token-{token_mint[:6]}"),
                symbol=update.get("symbol", token_mint[:4].upper()),
                price=price,
                liquidity_sol=liquidity_sol,
                market_cap_sol=float(update.get("market_cap_sol", 0.0) or 0.0),
                volume_24h_sol=float(update.get("volume_sol", 0.0) or 0.0),
                holder_count=int(update.get("holder_count", 0) or 0),
                bonding_curve_progress=float(update.get("bonding_curve_progress", 0.0) or 0.0),
                price_history=[price] if price else [],
                volume_history=[liquidity_sol] if liquidity_sol else [],
                created_at=datetime.utcnow(),
            )
        except Exception as exc:
            logger.debug("Failed to parse StreamOrders update", error=str(exc), update=update)
            return None

    async def _dispatch(self, token: TokenMarketData):
        """Fire all registered callbacks with the parsed token event."""
        if not self._callbacks:
            return
        tasks = [asyncio.create_task(cb(token)) for cb in self._callbacks]
        results = await asyncio.gather(*tasks, return_exceptions=True)
        for r in results:
            if isinstance(r, Exception):
                logger.warning("DataCollector callback error", error=str(r))
            else:
                self.events_dispatched += 1

    def get_stats(self) -> dict:
        return {
            "running": self._running,
            "callbacks": len(self._callbacks),
            "events_received": self.events_received,
            "events_dispatched": self.events_dispatched,
        }
