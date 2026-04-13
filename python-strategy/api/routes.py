from fastapi import APIRouter, Depends, HTTPException
from typing import List, Dict, Any, Optional
from pydantic import BaseModel
import structlog

from config import settings

logger = structlog.get_logger(__name__)
router = APIRouter()

# Reference to the strategy engine (injected via app state)
def get_engine():
    from main import app
    return app.state.engine


class OrderRequest(BaseModel):
    token_mint: str
    side: str
    amount_sol: float
    order_type: str = "MARKET"
    slippage_bps: int = 100
    strategy_name: str = "manual"


class StrategyConfigUpdate(BaseModel):
    strategy_name: str
    enabled: Optional[bool] = None
    buy_amount_sol: Optional[float] = None


@router.get("/health")
async def health():
    return {"status": "ok", "version": "1.0.0"}


@router.get("/portfolio")
async def get_portfolio(engine=Depends(get_engine)):
    """Get portfolio summary from Rust engine."""
    return await engine.grpc_client.get_portfolio_summary()


@router.get("/strategies")
async def get_strategies(engine=Depends(get_engine)):
    """Get stats for all running strategies."""
    return engine.get_strategy_stats()


@router.patch("/strategies/{strategy_name}")
async def update_strategy(
    strategy_name: str,
    update: StrategyConfigUpdate,
    engine=Depends(get_engine),
):
    """Enable/disable or configure a strategy."""
    for strategy in engine.strategies:
        if strategy.name == strategy_name:
            if update.enabled is not None:
                strategy.enabled = update.enabled
                logger.info("Strategy updated", name=strategy_name, enabled=update.enabled)
            if update.buy_amount_sol is not None:
                if hasattr(strategy, "buy_amount_sol"):
                    strategy.buy_amount_sol = update.buy_amount_sol
            return {"success": True, "strategy": strategy.get_stats()}
    raise HTTPException(status_code=404, detail=f"Strategy '{strategy_name}' not found")


@router.post("/orders")
async def submit_manual_order(order: OrderRequest, engine=Depends(get_engine)):
    """Manually submit an order via the Rust engine."""
    amount_lamports = int(order.amount_sol * 1_000_000_000)
    result = await engine.grpc_client.submit_order(
        token_mint=order.token_mint,
        side=order.side.upper(),
        amount=amount_lamports,
        order_type=order.order_type,
        slippage_bps=order.slippage_bps,
        strategy_name=order.strategy_name,
    )
    if not result.get("success"):
        raise HTTPException(status_code=400, detail=result.get("message", "Order failed"))
    return result


@router.get("/orders/{order_id}")
async def get_order_status(order_id: str, engine=Depends(get_engine)):
    """Get order status from Rust engine."""
    return await engine.grpc_client.get_order_status(order_id)


@router.delete("/orders/{order_id}")
async def cancel_order(order_id: str, engine=Depends(get_engine)):
    """Cancel an order."""
    return await engine.grpc_client.cancel_order(order_id)


@router.get("/tokens/{mint}")
async def get_token_info(mint: str, engine=Depends(get_engine)):
    """Get token info from Rust engine."""
    info = await engine.grpc_client.get_token_info(mint)
    if not info:
        raise HTTPException(status_code=404, detail="Token not found")
    return info


@router.get("/tokens")
async def get_tracked_tokens(engine=Depends(get_engine)):
    """Get all currently tracked tokens."""
    tokens = {}
    for mint, token in engine.tracked_tokens.items():
        tokens[mint] = {
            "mint": token.mint,
            "name": token.name,
            "symbol": token.symbol,
            "price": token.price,
            "liquidity_sol": token.liquidity_sol,
            "market_cap_sol": token.market_cap_sol,
            "volume_24h_sol": token.volume_24h_sol,
            "holder_count": token.holder_count,
            "bonding_curve_progress": token.bonding_curve_progress,
        }
    return tokens
