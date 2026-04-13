from typing import Optional
from datetime import datetime, timedelta
import structlog

from .base import BaseStrategy, TradeSignal, TokenMarketData
from ml import MLSignalGenerator, Signal
from ml.signal_generator import SignalDirection
from config import settings

logger = structlog.get_logger(__name__)


class SniperStrategy(BaseStrategy):
    """
    New token sniper strategy:
    Identifies and quickly buys newly launched tokens on Pump.fun
    before they gain momentum, targeting early bonding curve stages.
    """

    def __init__(self):
        super().__init__("sniper")
        self.ml_generator = MLSignalGenerator(settings.ml_model_path)
        self.max_token_age_minutes = 5
        self.min_liquidity_sol = settings.sniper_min_liquidity_sol
        self.max_market_cap_sol = settings.sniper_max_market_cap_sol
        self.buy_amount_sol = settings.sniper_buy_amount_sol

    def should_enter(self, token: TokenMarketData) -> bool:
        """Filter: only consider new tokens with low market cap."""
        if not settings.sniper_enabled:
            return False

        # Must have minimum liquidity
        if token.liquidity_sol < self.min_liquidity_sol:
            return False

        # Must be in early bonding curve
        if token.bonding_curve_progress > 30:
            return False

        # Market cap filter - we want early stage tokens
        if token.market_cap_sol > self.max_market_cap_sol:
            return False

        # Token should be new (if we have creation time)
        if token.created_at:
            age = datetime.utcnow() - token.created_at
            if age > timedelta(minutes=self.max_token_age_minutes):
                return False

        return True

    async def analyze(self, token: TokenMarketData) -> Optional[TradeSignal]:
        """Analyze new token for sniper opportunity."""
        if not self.should_enter(token):
            return None

        signal = self.ml_generator.generate_signal(
            token_mint=token.mint,
            price=token.price,
            price_history=token.price_history or [token.price],
            volume_history=token.volume_history or [token.volume_24h_sol],
            liquidity_sol=token.liquidity_sol,
            market_cap_sol=token.market_cap_sol,
            bonding_curve_progress=token.bonding_curve_progress,
            holder_count=token.holder_count,
        )

        if signal.direction != SignalDirection.BUY:
            return None

        if signal.confidence < settings.ml_confidence_threshold:
            logger.debug(
                "Sniper signal too low confidence",
                token=token.mint,
                confidence=signal.confidence,
            )
            return None

        # Calculate buy amount in lamports
        amount_lamports = int(self.buy_amount_sol * 1_000_000_000)

        return TradeSignal(
            token_mint=token.mint,
            side="BUY",
            amount_sol=self.buy_amount_sol,
            reason=f"Sniper: {signal.reason} (confidence={signal.confidence:.2f})",
            confidence=signal.confidence,
            strategy_name=self.name,
            slippage_bps=settings.sniper_slippage_bps,
            metadata={
                "strategy": "sniper",
                "ml_score": str(signal.ml_score),
                "market_cap": str(token.market_cap_sol),
                "bc_progress": str(token.bonding_curve_progress),
            },
        )
