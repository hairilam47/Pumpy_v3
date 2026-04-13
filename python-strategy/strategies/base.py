from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Optional, Dict, List
from datetime import datetime
import structlog

logger = structlog.get_logger(__name__)


@dataclass
class TradeSignal:
    token_mint: str
    side: str  # "BUY" or "SELL"
    amount_sol: float
    reason: str
    confidence: float
    strategy_name: str
    slippage_bps: int = 100
    metadata: Dict[str, str] = field(default_factory=dict)
    timestamp: datetime = field(default_factory=datetime.utcnow)


@dataclass
class TokenMarketData:
    mint: str
    name: str
    symbol: str
    price: float
    liquidity_sol: float
    market_cap_sol: float
    volume_24h_sol: float
    holder_count: int
    bonding_curve_progress: float
    price_history: List[float] = field(default_factory=list)
    volume_history: List[float] = field(default_factory=list)
    created_at: Optional[datetime] = None


class BaseStrategy(ABC):
    """Base class for all trading strategies."""

    def __init__(self, name: str):
        self.name = name
        self.enabled = True
        self.trades_executed = 0
        self.trades_won = 0
        self.total_pnl = 0.0
        self.logger = structlog.get_logger(f"strategy.{name}")

    @abstractmethod
    async def analyze(self, token: TokenMarketData) -> Optional[TradeSignal]:
        """Analyze token and return a trade signal if appropriate."""
        pass

    @abstractmethod
    def should_enter(self, token: TokenMarketData) -> bool:
        """Quick filter: should we analyze this token at all?"""
        pass

    def record_trade_result(self, won: bool, pnl_sol: float):
        """Record trade outcome for performance tracking."""
        self.trades_executed += 1
        if won:
            self.trades_won += 1
        self.total_pnl += pnl_sol

    @property
    def win_rate(self) -> float:
        if self.trades_executed == 0:
            return 0.0
        return self.trades_won / self.trades_executed * 100.0

    def get_stats(self) -> Dict:
        return {
            "name": self.name,
            "enabled": self.enabled,
            "trades_executed": self.trades_executed,
            "trades_won": self.trades_won,
            "win_rate": self.win_rate,
            "total_pnl_sol": self.total_pnl,
        }
