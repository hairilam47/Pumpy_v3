from fastapi import APIRouter, Depends, HTTPException, Header, Request
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
async def get_metrics(request: Request, engine=Depends(get_engine)):
    """
    Aggregate performance metrics including Sharpe ratio, max drawdown (%), and volatility.
    PnL series is sourced from persisted trades in the Express API so metrics survive restarts.
    """
    from strategy_engine import _compute_advanced_metrics

    base = engine.get_metrics()

    # Fetch persisted trade PnL from the Express API (DB-backed) to override in-memory
    # pnl_history which only accumulates from the current process session.
    try:
        express_url = os.environ.get("EXPRESS_API_URL", "http://localhost:8080")
        async with aiohttp.ClientSession() as session:
            async with session.get(
                f"{express_url}/api/bot/trades?limit=500",
                timeout=aiohttp.ClientTimeout(total=3),
            ) as resp:
                if resp.status == 200:
                    trades = await resp.json()
                    # Trades arrive newest-first (DESC order). Reverse to chronological
                    # so sequence-dependent metrics (max drawdown) are computed correctly.
                    db_pnl = [
                        float(t["pnlSol"])
                        for t in reversed(trades if isinstance(trades, list) else [])
                        if t.get("pnlSol") is not None
                    ]
                    if len(db_pnl) >= 2:
                        base.update(_compute_advanced_metrics(db_pnl))
    except Exception:
        pass

    return base


class BacktestRequest(BaseModel):
    strategy_name: str
    token_mints: Optional[List[str]] = None
    days: int = 7
    initial_sol: float = 10.0
    buy_amount_sol: Optional[float] = None
    stop_loss_pct: Optional[float] = None
    take_profit_pct: Optional[float] = None
    min_liquidity_sol: Optional[float] = None


@router.post("/backtest")
async def run_backtest(body: BacktestRequest, engine=Depends(get_engine)):
    """
    Run a historical backtest simulation.

    Data source (in priority order):
      1. Persistent token_metrics rows fetched from Express API (filtered by `days`)
      2. In-memory price_history from engine.tracked_tokens (fallback when DB is empty)

    Config overrides (buy_amount_sol, stop_loss_pct, take_profit_pct, min_liquidity_sol) are
    applied via an isolated dict — live strategy instances are NEVER mutated.

    Signal heuristic is strategy-specific:
      - sniper:   buy when bonding_curve_progress <= 30 and positive 1-bar return
      - momentum: buy when 5-bar slope is positive (short MA > long MA proxy)
      - default:  buy on any positive short-term momentum

    Returns: equity curve, total return, Sharpe ratio, max drawdown (%), win rate, trade count.
    """
    from strategy_engine import _compute_advanced_metrics
    from collections import defaultdict

    strategy = next((s for s in engine.strategies if s.name == body.strategy_name), None)
    if strategy is None:
        raise HTTPException(status_code=404, detail=f"Strategy '{body.strategy_name}' not found")

    # Build isolated config — never mutates the live strategy
    cfg: Dict[str, Any] = {
        "buy_amount_sol": body.buy_amount_sol if body.buy_amount_sol is not None else getattr(strategy, "buy_amount_sol", 0.1),
        "stop_loss_pct": body.stop_loss_pct if body.stop_loss_pct is not None else getattr(strategy, "stop_loss_pct", None),
        "take_profit_pct": body.take_profit_pct if body.take_profit_pct is not None else getattr(strategy, "take_profit_pct", None),
        "min_liquidity_sol": body.min_liquidity_sol if body.min_liquidity_sol is not None else getattr(strategy, "min_liquidity_sol", 0.0),
    }

    # ── 1. Fetch historical rows from the token_metrics DB table ──────────────
    # Build per-mint time-ordered price series from persistent snapshots.
    express_url = os.environ.get("EXPRESS_API_URL", "http://localhost:8080")
    db_price_series: Dict[str, List[float]] = defaultdict(list)
    db_meta: Dict[str, Dict] = {}

    try:
        params = f"days={body.days}&limit=5000"
        if body.token_mints:
            for m in body.token_mints[:10]:
                params += f"&mint={m}"
        async with aiohttp.ClientSession() as session:
            async with session.get(
                f"{express_url}/api/token-metrics?{params}",
                timeout=aiohttp.ClientTimeout(total=5),
            ) as resp:
                if resp.status == 200:
                    rows = await resp.json()
                    if isinstance(rows, list):
                        # Rows are returned newest-first; reverse to get chronological order
                        for row in reversed(rows):
                            mint = row.get("mint")
                            price = row.get("price")
                            if mint and price and price > 0:
                                db_price_series[mint].append(float(price))
                                if mint not in db_meta:
                                    db_meta[mint] = {
                                        "liquidity_sol": row.get("liquiditySol", 0.0) or 0.0,
                                        "bonding_curve_progress": row.get("bondingCurveProgress", 50.0) or 50.0,
                                    }
    except Exception as exc:
        logger.debug("Backtest: could not fetch token_metrics from DB", error=str(exc))

    # ── 2. Fall back to in-memory price_history if DB returned nothing ────────
    if not db_price_series:
        for mint, token in list(engine.tracked_tokens.items())[:10]:
            if token.price_history and len(token.price_history) >= 4:
                db_price_series[mint] = list(token.price_history)
                db_meta[mint] = {
                    "liquidity_sol": token.liquidity_sol,
                    "bonding_curve_progress": token.bonding_curve_progress,
                }

    # ── 3. Filter to requested mints ─────────────────────────────────────────
    if body.token_mints:
        target = set(body.token_mints)
        db_price_series = {m: v for m, v in db_price_series.items() if m in target}

    sol = body.initial_sol
    equity_curve: List[float] = [sol]
    pnl_series: List[float] = []
    wins = 0
    trades = 0

    # ── 4. Strategy-specific signal heuristic ────────────────────────────────
    def _should_enter(mint: str, history: List[float], i: int) -> bool:
        """Return True if the strategy would signal BUY at position i."""
        if history[i - 1] <= 0:
            return False
        meta = db_meta.get(mint, {})
        liq = meta.get("liquidity_sol", 0.0)
        bc = meta.get("bonding_curve_progress", 50.0)

        if liq < cfg["min_liquidity_sol"]:
            return False

        if body.strategy_name == "sniper":
            # Sniper enters on fresh early-stage tokens with upward momentum
            if bc > 30:
                return False
            lookback = history[max(0, i - 3):i]
            return len(lookback) >= 2 and lookback[-1] > lookback[0]

        elif body.strategy_name == "momentum":
            # Momentum enters when short MA > long MA (proxy for trend)
            short = history[max(0, i - 5):i]
            long_ = history[max(0, i - 20):i]
            if len(short) < 2:
                return False
            short_ma = sum(short) / len(short)
            long_ma = sum(long_) / len(long_)
            return short_ma > long_ma

        else:
            # Generic: positive momentum over last 5 bars
            lookback = history[max(0, i - 5):i]
            return len(lookback) >= 2 and lookback[-1] > lookback[0]

    # ── 5. Replay each token's price series ──────────────────────────────────
    for mint, history in db_price_series.items():
        if len(history) < 4:
            continue
        for i in range(1, len(history)):
            if not _should_enter(mint, history, i):
                continue
            entry_price = history[i - 1]
            exit_price = history[i]
            if entry_price <= 0:
                continue
            return_pct = (exit_price - entry_price) / entry_price
            if cfg["stop_loss_pct"] is not None:
                return_pct = max(return_pct, -cfg["stop_loss_pct"] / 100.0)
            if cfg["take_profit_pct"] is not None:
                return_pct = min(return_pct, cfg["take_profit_pct"] / 100.0)
            pnl = cfg["buy_amount_sol"] * return_pct
            sol += pnl
            pnl_series.append(pnl)
            trades += 1
            if pnl > 0:
                wins += 1
            equity_curve.append(round(sol, 6))

    # ── 6. Compute advanced metrics ───────────────────────────────────────────
    win_rate = (wins / trades * 100.0) if trades > 0 else 0.0
    total_return_pct = (sol - body.initial_sol) / body.initial_sol * 100.0
    advanced = _compute_advanced_metrics(pnl_series)

    return {
        "strategy": body.strategy_name,
        "days": body.days,
        "initial_sol": body.initial_sol,
        "final_sol": round(sol, 6),
        "simulated_pnl_sol": round(sol - body.initial_sol, 6),
        "total_return_pct": round(total_return_pct, 4),
        "total_trades": trades,
        "wins": wins,
        "win_rate": round(win_rate, 2),
        "sharpe_ratio": advanced["sharpe_ratio"],
        "max_drawdown_pct": advanced["max_drawdown_pct"],
        "volatility": advanced["volatility"],
        "equity_curve": equity_curve[-200:],
        "data_source": "db" if db_meta and any(
            mint in db_meta for mint in db_price_series
        ) else "in_memory",
    }


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
async def set_strategy_preset(
    body: PresetUpdate,
    engine=Depends(get_engine),
    x_admin_key: Optional[str] = Header(default=None),
):
    """
    Set the strategy risk preset for a wallet.
    Writes the preset to wallet_config via the Express API and applies
    preset parameters to all active strategies immediately.
    Allowed values: conservative | balanced | aggressive.
    """
    # Require the same admin key that guards Express mutation routes.
    server_admin_key = os.environ.get("ADMIN_API_KEY", "")
    if not server_admin_key or x_admin_key != server_admin_key:
        raise HTTPException(status_code=401, detail="Admin key required")

    if body.preset not in PRESETS:
        raise HTTPException(
            status_code=400,
            detail=f"preset must be one of: {', '.join(PRESETS.keys())}",
        )

    params = PRESETS[body.preset]

    # Persist to wallet_config via Express API. Fail loudly if the write does
    # not succeed so callers cannot assume the preset is stored when it isn't.
    api_base = os.environ.get("EXPRESS_API_URL", "http://localhost:8080")
    admin_key = server_admin_key
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
