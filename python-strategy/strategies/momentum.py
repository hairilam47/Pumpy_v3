from typing import Optional, Dict, List
from collections import defaultdict, deque
from datetime import datetime, timedelta
import numpy as np
import structlog

from .base import BaseStrategy, TradeSignal, TokenMarketData
from ml import MLSignalGenerator
from ml.signal_generator import SignalDirection
from config import settings

logger = structlog.get_logger(__name__)


class MomentumStrategy(BaseStrategy):
    """
    Momentum trading strategy:
    Identifies tokens with strong upward price and volume momentum
    and enters positions to ride the wave.
    """

    def __init__(self):
        super().__init__("momentum")
        self.ml_generator = MLSignalGenerator(settings.ml_model_path)
        self.price_windows: Dict[str, deque] = defaultdict(lambda: deque(maxlen=100))
        self.volume_windows: Dict[str, deque] = defaultdict(lambda: deque(maxlen=100))
        self.last_signal_time: Dict[str, datetime] = {}
        self.cooldown_seconds = 60
        self.buy_amount_sol = settings.momentum_buy_amount_sol

    def should_enter(self, token: TokenMarketData) -> bool:
        """Filter: only consider tokens with sufficient data and liquidity."""
        if not settings.momentum_enabled:
            return False

        if token.liquidity_sol < 2.0:
            return False

        if token.market_cap_sol < 5.0:
            return False

        if token.bonding_curve_progress > 90:
            return False

        # Check cooldown
        last_signal = self.last_signal_time.get(token.mint)
        if last_signal:
            elapsed = (datetime.utcnow() - last_signal).total_seconds()
            if elapsed < self.cooldown_seconds:
                return False

        return True

    def update_price_data(self, mint: str, price: float, volume: float):
        """Update internal price and volume tracking."""
        self.price_windows[mint].append(price)
        self.volume_windows[mint].append(volume)

    def calculate_momentum_score(self, prices: List[float], volumes: List[float]) -> float:
        """Calculate composite momentum score (0-1)."""
        if len(prices) < 5:
            return 0.0

        prices_arr = np.array(prices)
        vols_arr = np.array(volumes) if volumes else np.ones(len(prices))

        # Price momentum (last 5 vs last 20)
        short_ma = np.mean(prices_arr[-5:])
        long_ma = np.mean(prices_arr[-20:]) if len(prices_arr) >= 20 else np.mean(prices_arr)
        price_momentum = (short_ma / (long_ma + 1e-10)) - 1.0

        # Volume momentum (last 5 vs average)
        recent_vol = np.mean(vols_arr[-5:])
        avg_vol = np.mean(vols_arr)
        vol_momentum = (recent_vol / (avg_vol + 1e-10)) - 1.0

        # Trend consistency (fraction of positive returns)
        returns = np.diff(prices_arr)
        positive_frac = np.sum(returns > 0) / max(len(returns), 1)

        # Composite score
        score = 0.4 * np.tanh(price_momentum * 10) + \
                0.3 * np.tanh(vol_momentum) + \
                0.3 * (positive_frac - 0.5) * 2

        return float(max(0.0, min(1.0, (score + 1.0) / 2.0)))

    async def analyze(self, token: TokenMarketData) -> Optional[TradeSignal]:
        """Analyze token for momentum opportunity."""
        if not self.should_enter(token):
            return None

        # Update internal tracking
        self.update_price_data(
            token.mint,
            token.price,
            token.volume_24h_sol,
        )

        prices = list(self.price_windows.get(token.mint, [])) or token.price_history
        volumes = list(self.volume_windows.get(token.mint, [])) or token.volume_history

        # Calculate momentum score
        momentum_score = self.calculate_momentum_score(prices, volumes)

        # Get ML signal
        signal = self.ml_generator.generate_signal(
            token_mint=token.mint,
            price=token.price,
            price_history=prices,
            volume_history=volumes,
            liquidity_sol=token.liquidity_sol,
            market_cap_sol=token.market_cap_sol,
            bonding_curve_progress=token.bonding_curve_progress,
            holder_count=token.holder_count,
        )

        # Combined score
        combined_score = 0.6 * signal.ml_score + 0.4 * momentum_score

        # Volume spike check
        volume_threshold = settings.momentum_volume_threshold
        has_volume_spike = token.volume_24h_sol > volume_threshold

        # Price change check
        price_change = 0.0
        if len(prices) >= 2:
            price_change = (prices[-1] - prices[-2]) / (prices[-2] + 1e-10)

        threshold = settings.momentum_price_change_threshold
        has_price_momentum = price_change > threshold

        if not (has_volume_spike or has_price_momentum):
            if combined_score < 0.7:
                return None

        if combined_score < 0.55:
            return None

        self.last_signal_time[token.mint] = datetime.utcnow()

        side = "BUY" if signal.direction == SignalDirection.BUY else "SELL"
        if signal.direction == SignalDirection.HOLD:
            return None

        return TradeSignal(
            token_mint=token.mint,
            side=side,
            amount_sol=self.buy_amount_sol,
            reason=(
                f"Momentum: score={combined_score:.2f} "
                f"price_chg={price_change*100:.1f}% "
                f"vol={token.volume_24h_sol:.1f} SOL"
            ),
            confidence=combined_score,
            strategy_name=self.name,
            slippage_bps=settings.momentum_slippage_bps,
            metadata={
                "strategy": "momentum",
                "momentum_score": str(momentum_score),
                "ml_score": str(signal.ml_score),
                "price_change": str(price_change),
            },
        )
