"""
PumpFunDataCollector — subscribes to the Rust engine's StreamOrders gRPC stream
and surfaces order-lifecycle events that indicate a new token has been detected/sniped.

Design notes
------------
The Rust `StreamOrders` RPC emits `OrderUpdate` messages describing order lifecycle
(PENDING → FILLED / FAILED / CANCELLED).  Token-discovery events that are useful to
strategies are updates where:
  - `status` is one of the new-token statuses: "NEW", "DETECTED", "PENDING"
  - `token_mint` is a non-empty field set by the Rust engine when it submits an order
    for a newly discovered token.

Critically: we NEVER use `order_id` as a token mint.  `order_id` is a UUID and is
not a valid Solana public key.  If `token_mint` is absent or empty in the update,
we discard the event rather than fabricating an invalid mint address.
"""

import asyncio
from datetime import datetime
from typing import Callable, Awaitable, List, Optional
import structlog

from strategies.base import TokenMarketData

logger = structlog.get_logger(__name__)

TokenEventCallback = Callable[[TokenMarketData], Awaitable[None]]

NEW_TOKEN_STATUSES = frozenset({"NEW", "DETECTED", "PENDING"})

_SOLANA_PUBKEY_MIN_LEN = 32
_SOLANA_PUBKEY_MAX_LEN = 44


def _is_valid_mint(value: str) -> bool:
    """
    Quick sanity check: a Solana base-58 public key is 32-44 chars and contains
    only base-58 alphabet characters.  UUIDs (order_id) contain hyphens and are
    never valid mints.
    """
    if not value or "-" in value:
        return False
    if not (_SOLANA_PUBKEY_MIN_LEN <= len(value) <= _SOLANA_PUBKEY_MAX_LEN):
        return False
    return True


class PumpFunDataCollector:
    """
    Streams new-token events from the Rust engine's StreamOrders gRPC stream
    and distributes them to registered callbacks (i.e. active strategies).

    Only events with an explicit `token_mint` field and a new-token status
    are forwarded; all other order lifecycle updates are silently ignored.

    Usage::

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
        self.events_forwarded = 0
        self.events_discarded = 0

    def register_callback(self, callback: TokenEventCallback):
        """Register an async callback that receives TokenMarketData on each new-token event."""
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
        """Stop the streaming loop and wait for it to exit cleanly."""
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
            events_forwarded=self.events_forwarded,
            events_discarded=self.events_discarded,
        )

    async def _stream_loop(self):
        """
        Inner loop: subscribe to StreamOrders.
        When the gRPC client is not connected, sleep with exponential backoff
        instead of spinning.  On stream error, also back off before retrying.
        """
        backoff = 5.0
        while self._running:
            if not self.grpc_client.connected:
                await asyncio.sleep(backoff)
                backoff = min(backoff * 1.5, 60.0)
                continue

            backoff = 5.0
            try:
                async for update in self.grpc_client.stream_orders():
                    if not self._running:
                        break
                    self.events_received += 1
                    token = self._parse_update(update)
                    if token is not None:
                        await self._dispatch(token)
                    else:
                        self.events_discarded += 1
            except asyncio.CancelledError:
                break
            except Exception as exc:
                logger.warning(
                    "DataCollector stream error — retrying in %.0fs" % backoff,
                    error=str(exc),
                )
                await asyncio.sleep(backoff)
                backoff = min(backoff * 1.5, 60.0)

    def _parse_update(self, update: dict) -> Optional[TokenMarketData]:
        """
        Convert a raw StreamOrders update dict into a TokenMarketData object.

        Rules:
        1. Only accept events whose `status` indicates a new token discovery.
        2. `token_mint` MUST be present and must look like a valid Solana pubkey.
           Never use `order_id` as a mint — it is a UUID, not a public key.
        3. All numeric fields default to 0.0/0 when absent or None.
        """
        status = (update.get("status") or "").upper()
        if status not in NEW_TOKEN_STATUSES:
            return None

        token_mint = update.get("token_mint") or ""
        if not _is_valid_mint(token_mint):
            logger.debug(
                "DataCollector discarding event: missing or invalid token_mint",
                status=status,
                order_id=update.get("order_id", ""),
            )
            return None

        try:
            price = float(update.get("executed_price") or 0.0)
            amount_lamports = int(update.get("executed_amount") or 0)
            liquidity_sol = amount_lamports / 1_000_000_000

            return TokenMarketData(
                mint=token_mint,
                name=update.get("name") or f"Token-{token_mint[:6]}",
                symbol=update.get("symbol") or token_mint[:4].upper(),
                price=price,
                liquidity_sol=liquidity_sol,
                market_cap_sol=float(update.get("market_cap_sol") or 0.0),
                volume_24h_sol=float(update.get("volume_sol") or 0.0),
                holder_count=int(update.get("holder_count") or 0),
                bonding_curve_progress=float(update.get("bonding_curve_progress") or 0.0),
                price_history=[price] if price > 0 else [],
                volume_history=[liquidity_sol] if liquidity_sol > 0 else [],
                created_at=datetime.utcnow(),
            )
        except Exception as exc:
            logger.debug(
                "DataCollector failed to build TokenMarketData",
                error=str(exc),
                update=update,
            )
            return None

    async def _dispatch(self, token: TokenMarketData):
        """Fire all registered callbacks with the parsed token event."""
        if not self._callbacks:
            return
        tasks = [asyncio.create_task(cb(token)) for cb in self._callbacks]
        results = await asyncio.gather(*tasks, return_exceptions=True)
        forwarded = 0
        for r in results:
            if isinstance(r, Exception):
                logger.warning("DataCollector callback error", error=str(r))
            else:
                forwarded += 1
        self.events_forwarded += forwarded

    def get_stats(self) -> dict:
        return {
            "running": self._running,
            "callbacks": len(self._callbacks),
            "events_received": self.events_received,
            "events_forwarded": self.events_forwarded,
            "events_discarded": self.events_discarded,
        }
