from fastapi import APIRouter, Depends, HTTPException, Request
from typing import List, Dict, Any, Optional
from pydantic import BaseModel
import structlog
import os
import aiohttp

from config import settings

logger = structlog.get_logger(__name__)
router = APIRouter()

# Reference to the strategy engine (injected via app state)
def get_engine(request: Request):
    return request.app.state.engine


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
    slippage_bps: Optional[int] = None
    take_profit_pct: Optional[float] = None
    stop_loss_pct: Optional[float] = None
    trailing_stop_pct: Optional[float] = None
    min_liquidity_sol: Optional[float] = None


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
            if update.slippage_bps is not None and hasattr(strategy, "slippage_bps"):
                strategy.slippage_bps = update.slippage_bps
            if update.take_profit_pct is not None and hasattr(strategy, "take_profit_pct"):
                strategy.take_profit_pct = update.take_profit_pct
            if update.stop_loss_pct is not None and hasattr(strategy, "stop_loss_pct"):
                strategy.stop_loss_pct = update.stop_loss_pct
            if update.trailing_stop_pct is not None and hasattr(strategy, "trailing_stop_pct"):
                strategy.trailing_stop_pct = update.trailing_stop_pct
            if update.min_liquidity_sol is not None and hasattr(strategy, "min_liquidity_sol"):
                strategy.min_liquidity_sol = update.min_liquidity_sol
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


@router.get("/metrics")
async def get_metrics(engine=Depends(get_engine)):
    """
    Aggregate performance metrics: win rate, PnL, open positions, trade count.
    Consumed by the dashboard via the Express API server.
    """
    return engine.get_metrics()


@router.post("/strategy/activate")
async def activate_strategy(body: StrategyConfigUpdate, engine=Depends(get_engine)):
    """Activate or deactivate a strategy by name."""
    for strategy in engine.strategies:
        if strategy.name == body.strategy_name:
            if body.enabled is not None:
                strategy.enabled = body.enabled
            return {"success": True, "strategy": strategy.get_stats()}
    raise HTTPException(status_code=404, detail=f"Strategy '{body.strategy_name}' not found")


@router.post("/strategy/config")
async def update_strategy_config(body: StrategyConfigUpdate, engine=Depends(get_engine)):
    """Update strategy configuration parameters."""
    for strategy in engine.strategies:
        if strategy.name == body.strategy_name:
            if body.enabled is not None:
                strategy.enabled = body.enabled
            if body.buy_amount_sol is not None and hasattr(strategy, "buy_amount_sol"):
                strategy.buy_amount_sol = body.buy_amount_sol
            return {"success": True, "strategy": strategy.get_stats()}
    raise HTTPException(status_code=404, detail=f"Strategy '{body.strategy_name}' not found")


PRESETS = {
    "conservative": {
        "risk_per_trade_sol": 0.05,
        "stop_loss_pct": 5,
        "take_profit_pct": 20,
        "max_positions": 2,
    },
    "balanced": {
        "risk_per_trade_sol": 0.15,
        "stop_loss_pct": 10,
        "take_profit_pct": 50,
        "max_positions": 5,
    },
    "aggressive": {
        "risk_per_trade_sol": 0.5,
        "stop_loss_pct": 20,
        "take_profit_pct": 100,
        "max_positions": 10,
    },
}


class PresetUpdate(BaseModel):
    preset: str
    wallet_id: str = "wallet_001"


@router.put("/strategy/preset")
async def set_strategy_preset(body: PresetUpdate, engine=Depends(get_engine)):
    """
    Set the strategy risk preset for a wallet.
    Writes the preset to wallet_config via the Express API and applies
    preset parameters to all active strategies immediately.
    Allowed values: conservative | balanced | aggressive.
    """
    if body.preset not in PRESETS:
        raise HTTPException(
            status_code=400,
            detail=f"preset must be one of: {', '.join(PRESETS.keys())}",
        )

    params = PRESETS[body.preset]

    # Persist to wallet_config via Express API. Fail loudly if the write does
    # not succeed so callers cannot assume the preset is stored when it isn't.
    api_base = os.environ.get("EXPRESS_API_URL", "http://localhost:8080")
    admin_key = os.environ.get("ADMIN_API_KEY", "")
    try:
        async with aiohttp.ClientSession() as session:
            async with session.put(
                f"{api_base}/api/wallets/{body.wallet_id}/config",
                json={"strategy_preset": body.preset},
                headers={"X-Admin-Key": admin_key, "Content-Type": "application/json"},
                timeout=aiohttp.ClientTimeout(total=5),
            ) as resp:
                if resp.status not in (200, 201, 204):
                    text = await resp.text()
                    logger.error("Failed to persist preset to wallet_config", status=resp.status, body=text)
                    raise HTTPException(
                        status_code=502,
                        detail=f"Failed to persist preset to wallet_config (status {resp.status}): {text[:200]}",
                    )
    except HTTPException:
        raise
    except Exception as exc:
        logger.error("Could not reach Express API for preset persistence", error=str(exc))
        raise HTTPException(
            status_code=502,
            detail=f"Could not persist preset — Express API unreachable: {exc}",
        )

    for strategy in engine.strategies:
        if hasattr(strategy, "buy_amount_sol"):
            strategy.buy_amount_sol = params["risk_per_trade_sol"]
        if hasattr(strategy, "stop_loss_pct"):
            strategy.stop_loss_pct = float(params["stop_loss_pct"])
        if hasattr(strategy, "take_profit_pct"):
            strategy.take_profit_pct = float(params["take_profit_pct"])

    logger.info("Strategy preset applied", preset=body.preset, wallet_id=body.wallet_id, params=params)
    return {
        "success": True,
        "preset": body.preset,
        "wallet_id": body.wallet_id,
        "params": params,
    }


@router.get("/strategy/preset")
async def get_preset_definitions():
    """Return the preset definitions (read-only reference)."""
    return PRESETS
