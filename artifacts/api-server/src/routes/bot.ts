import { Router, type Request, type Response } from "express";
import { db, tradesTable, strategiesTable } from "@workspace/db";
import { desc } from "drizzle-orm";
import { grpcBot } from "../lib/grpc-client";

const router = Router();

const PYTHON_API = process.env.PYTHON_API_URL ?? "http://localhost:8001";

async function fetchPython(path: string, init?: RequestInit): Promise<unknown | null> {
  try {
    const resp = await fetch(`${PYTHON_API}${path}`, {
      signal: AbortSignal.timeout(4000),
      ...init,
    });
    if (!resp.ok) return null;
    return await resp.json();
  } catch {
    return null;
  }
}

// ─── Portfolio ────────────────────────────────────────────────────────────────

// GET /api/bot/portfolio  →  Rust gRPC first, Python fallback, then static mock
router.get("/bot/portfolio", async (_req: Request, res: Response) => {
  try {
    // 1. Try Rust gRPC
    try {
      const grpc = await grpcBot.getPortfolioSummary();
      res.json({
        totalValueSol: grpc.total_value_sol,
        cashBalanceSol: grpc.cash_balance_sol,
        positionsValueSol: grpc.positions_value_sol,
        dailyPnlSol: grpc.daily_pnl_sol,
        totalPnlSol: grpc.total_pnl_sol,
        openPositionsCount: grpc.open_positions_count,
        winRate: grpc.win_rate,
        source: "rust",
      });
      return;
    } catch { /* fall through */ }

    // 2. Try Python FastAPI
    const pyData = await fetchPython("/api/portfolio") as Record<string, unknown> | null;
    if (pyData) {
      res.json({ ...pyData, source: "python" });
      return;
    }

    // 3. Static mock
    res.json({
      totalValueSol: 10.0,
      cashBalanceSol: 9.5,
      positionsValueSol: 0.5,
      dailyPnlSol: 0.0,
      totalPnlSol: 0.0,
      openPositionsCount: 0,
      winRate: 0,
      source: "mock",
    });
  } catch {
    res.status(500).json({ error: "Failed to get portfolio" });
  }
});

// ─── Trades ───────────────────────────────────────────────────────────────────

// GET /api/bot/trades  →  DB first, then mock
router.get("/bot/trades", async (req: Request, res: Response) => {
  try {
    const limit = Math.min(parseInt(String(req.query.limit ?? "50")), 200);
    const strategy = req.query.strategy as string | undefined;

    try {
      const query = db.select().from(tradesTable).orderBy(desc(tradesTable.createdAt)).limit(limit);
      const rows = await query;
      res.json(rows);
      return;
    } catch { /* fall through */ }

    res.json(generateMockTrades(limit, strategy));
  } catch {
    res.status(500).json({ error: "Failed to list trades" });
  }
});

// ─── Orders ───────────────────────────────────────────────────────────────────

// POST /api/bot/orders  →  Rust gRPC SubmitOrder first, Python fallback
router.post("/bot/orders", async (req: Request, res: Response) => {
  try {
    const body = req.body as {
      tokenMint?: string;
      side?: string;
      amountSol?: number;
      orderType?: string;
      slippageBps?: number;
      strategyName?: string;
    };

    if (!body.tokenMint || !body.side || !body.amountSol) {
      res.status(400).json({ error: "Missing required fields: tokenMint, side, amountSol" });
      return;
    }

    // 1. Rust gRPC
    try {
      const grpcResp = await grpcBot.submitOrder({
        token_mint: body.tokenMint,
        order_type: body.orderType ?? "MARKET",
        side: body.side,
        amount: Math.round(body.amountSol * 1_000_000_000),
        slippage_bps: body.slippageBps ?? 100,
        strategy_name: body.strategyName ?? "manual",
      });
      res.json({
        orderId: grpcResp.order_id,
        success: grpcResp.success,
        message: grpcResp.message,
        source: "rust",
      });
      return;
    } catch { /* fall through */ }

    // 2. Python fallback
    const pyData = await fetchPython("/api/orders", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        token_mint: body.tokenMint,
        side: body.side,
        amount_sol: body.amountSol,
        order_type: body.orderType ?? "MARKET",
        slippage_bps: body.slippageBps ?? 100,
        strategy_name: body.strategyName ?? "manual",
      }),
    }) as Record<string, unknown> | null;

    res.json(pyData ?? { success: false, orderId: "", message: "Engine not available" });
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : "Invalid request";
    res.status(400).json({ error: msg });
  }
});

// GET /api/bot/orders/:orderId  →  Rust gRPC GetOrderStatus first, Python fallback
router.get("/bot/orders/:orderId", async (req: Request, res: Response) => {
  try {
    const orderId = Array.isArray(req.params.orderId) ? req.params.orderId[0]! : req.params.orderId;

    // 1. Rust gRPC
    try {
      const status = await grpcBot.getOrderStatus(orderId);
      res.json({
        orderId: status.order_id,
        status: status.status,
        signature: status.signature || null,
        error: status.error || null,
        executedAt: status.executed_at ?? null,
        source: "rust",
      });
      return;
    } catch { /* fall through */ }

    // 2. Python fallback
    const pyData = await fetchPython(`/api/orders/${orderId}`) as Record<string, unknown> | null;
    if (pyData) {
      res.json(pyData);
      return;
    }

    res.status(404).json({ error: "Order not found" });
  } catch {
    res.status(404).json({ error: "Order not found" });
  }
});

// DELETE /api/bot/orders/:orderId  →  Rust gRPC CancelOrder first, Python fallback
router.delete("/bot/orders/:orderId", async (req: Request, res: Response) => {
  try {
    const orderId = Array.isArray(req.params.orderId) ? req.params.orderId[0]! : req.params.orderId;

    // 1. Rust gRPC
    try {
      const resp = await grpcBot.cancelOrder(orderId);
      res.json({ success: resp.success, message: resp.message, source: "rust" });
      return;
    } catch { /* fall through */ }

    // 2. Python fallback
    const pyData = await fetchPython(`/api/orders/${orderId}`, { method: "DELETE" }) as Record<string, unknown> | null;
    res.json(pyData ?? { success: true, message: "Cancelled (offline)" });
  } catch {
    res.status(500).json({ error: "Cancel failed" });
  }
});

// ─── Strategies ───────────────────────────────────────────────────────────────

// GET /api/bot/strategies  →  Python first (owns strategy state), then DB, then mock
router.get("/bot/strategies", async (_req: Request, res: Response) => {
  try {
    const pyData = await fetchPython("/api/strategies") as unknown[] | null;
    if (Array.isArray(pyData)) {
      const mapped = pyData.map((s: unknown) => {
        const strategy = s as Record<string, unknown>;
        return {
          name: strategy.name,
          enabled: strategy.enabled,
          tradesExecuted: strategy.trades_executed ?? 0,
          tradesWon: strategy.trades_won ?? 0,
          winRate: strategy.win_rate ?? 0,
          totalPnlSol: strategy.total_pnl_sol ?? 0,
          buyAmountSol: strategy.buy_amount_sol,
        };
      });
      res.json(mapped);
      return;
    }

    try {
      const rows = await db.select().from(strategiesTable);
      res.json(rows.map((r) => ({
        name: r.name,
        enabled: r.enabled,
        tradesExecuted: r.tradesExecuted,
        tradesWon: r.tradesWon,
        winRate: r.tradesExecuted > 0 ? (r.tradesWon / r.tradesExecuted) * 100 : 0,
        totalPnlSol: r.totalPnlSol,
        buyAmountSol: r.buyAmountSol,
      })));
      return;
    } catch { /* ignore */ }

    res.json([
      { name: "sniper", enabled: true, tradesExecuted: 0, tradesWon: 0, winRate: 0, totalPnlSol: 0, buyAmountSol: 0.05 },
      { name: "momentum", enabled: true, tradesExecuted: 0, tradesWon: 0, winRate: 0, totalPnlSol: 0, buyAmountSol: 0.1 },
    ]);
  } catch {
    res.status(500).json({ error: "Failed to list strategies" });
  }
});

// PATCH /api/bot/strategies/:strategyName  →  Python owns strategy config
router.patch("/bot/strategies/:strategyName", async (req: Request, res: Response) => {
  try {
    const { strategyName } = req.params;
    const body = req.body as {
      enabled?: boolean;
      buyAmountSol?: number;
      slippageBps?: number;
      takeProfitPct?: number;
      stopLossPct?: number;
      trailingStopPct?: number;
      minLiquiditySol?: number;
    };

    const pyData = await fetchPython(`/api/strategies/${strategyName}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        strategy_name: strategyName,
        enabled: body.enabled,
        buy_amount_sol: body.buyAmountSol,
        slippage_bps: body.slippageBps,
        take_profit_pct: body.takeProfitPct,
        stop_loss_pct: body.stopLossPct,
        trailing_stop_pct: body.trailingStopPct,
        min_liquidity_sol: body.minLiquiditySol,
      }),
    }) as Record<string, unknown> | null;

    if (pyData?.strategy) {
      res.json(pyData.strategy);
      return;
    }

    res.json({ name: strategyName, enabled: body.enabled ?? true, tradesExecuted: 0, tradesWon: 0, winRate: 0, totalPnlSol: 0 });
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : "Update failed";
    res.status(400).json({ error: msg });
  }
});

// ─── Tokens ───────────────────────────────────────────────────────────────────

// GET /api/bot/tokens  →  Python owns token cache
router.get("/bot/tokens", async (_req: Request, res: Response) => {
  try {
    const pyData = await fetchPython("/api/tokens") as Record<string, unknown> | null;
    if (pyData && typeof pyData === "object") {
      const tokens = Object.values(pyData as Record<string, Record<string, unknown>>).map((t) => ({
        mint: t.mint,
        name: t.name,
        symbol: t.symbol,
        price: t.price ?? 0,
        liquiditySol: t.liquidity_sol ?? 0,
        marketCapSol: t.market_cap_sol ?? 0,
        volume24hSol: t.volume_24h_sol,
        holderCount: t.holder_count,
        bondingCurveProgress: t.bonding_curve_progress ?? 0,
        mlScore: t.ml_score ?? t.sniper_score ?? null,
        detectedAt: t.detected_at ?? t.created_at ?? null,
        actionTaken: t.action_taken ?? t.action ?? null,
      }));
      res.json(tokens);
      return;
    }
    res.json([]);
  } catch {
    res.status(500).json({ error: "Failed to list tokens" });
  }
});

// GET /api/bot/tokens/:mint  →  Rust gRPC GetTokenInfo first, Python fallback
router.get("/bot/tokens/:mint", async (req: Request, res: Response) => {
  try {
    const mint = Array.isArray(req.params.mint) ? req.params.mint[0]! : req.params.mint;

    // 1. Rust gRPC
    try {
      const info = await grpcBot.getTokenInfo(mint);
      res.json({
        mint: info.mint,
        name: info.name,
        symbol: info.symbol,
        price: info.price,
        liquiditySol: info.liquidity_sol,
        marketCapSol: info.market_cap_sol,
        volume24hSol: info.volume_24h_sol,
        holderCount: info.holder_count,
        bondingCurveProgress: info.bonding_curve_progress,
        source: "rust",
      });
      return;
    } catch { /* fall through */ }

    // 2. Python fallback
    const pyData = await fetchPython(`/api/tokens/${mint}`) as Record<string, unknown> | null;
    if (!pyData) {
      res.status(404).json({ error: "Token not found" });
      return;
    }
    res.json({
      mint: pyData.mint,
      name: pyData.name,
      symbol: pyData.symbol,
      price: pyData.price ?? 0,
      liquiditySol: pyData.liquidity_sol ?? 0,
      marketCapSol: pyData.market_cap_sol ?? 0,
      volume24hSol: pyData.volume_24h_sol,
      holderCount: pyData.holder_count,
      bondingCurveProgress: pyData.bonding_curve_progress ?? 0,
      source: "python",
    });
  } catch {
    res.status(404).json({ error: "Token not found" });
  }
});

// ─── Metrics ──────────────────────────────────────────────────────────────────

// GET /api/bot/metrics  →  Python owns Prometheus-style metrics
router.get("/bot/metrics", async (_req: Request, res: Response) => {
  try {
    const pyData = await fetchPython("/api/metrics") as Record<string, unknown> | null;
    res.json(pyData ?? {
      ordersSubmitted: 0,
      ordersExecuted: 0,
      ordersFailed: 0,
      ordersPending: 0,
      jitoLanded: 0,
      sandwichAttacks: 0,
      avgExecutionMs: 0,
      rpcErrorRate: 0,
      tokensDiscovered: 0,
      tokensSniped: 0,
    });
  } catch {
    res.status(500).json({ error: "Failed to get metrics" });
  }
});

// ─── Status ───────────────────────────────────────────────────────────────────

// GET /api/bot/status  →  probe Rust gRPC + Python health
router.get("/bot/status", async (_req: Request, res: Response) => {
  try {
    let rustConnected = false;
    try {
      await grpcBot.getPortfolioSummary();
      rustConnected = true;
    } catch { /* Rust engine not running */ }

    const pyHealth = await fetchPython("/api/health");
    const pyConnected = !!pyHealth;

    res.json({
      running: rustConnected || pyConnected,
      rustEngineConnected: rustConnected,
      pythonEngineRunning: pyConnected,
      walletAddress: process.env.WALLET_ADDRESS ?? "",
      solBalance: 0,
      activeStrategies: ["sniper", "momentum"],
      environment: process.env.NODE_ENV ?? "development",
      uptime: process.uptime(),
    });
  } catch {
    res.json({
      running: false,
      rustEngineConnected: false,
      pythonEngineRunning: false,
      walletAddress: "",
      solBalance: 0,
      activeStrategies: [],
      environment: "development",
      uptime: 0,
    });
  }
});

// ─── MEV Stats ───────────────────────────────────────────────────────────────

router.get("/bot/mev-stats", async (_req: Request, res: Response) => {
  try {
    // Derive MEV stats from the Python metrics endpoint (which aggregates gRPC engine data)
    const pyMetrics = await fetchPython("/api/metrics") as Record<string, unknown> | null;
    const jitoEnabled = !!process.env.JITO_BUNDLE_URL;

    if (pyMetrics) {
      const submitted = Number(pyMetrics.jitoLanded ?? pyMetrics.bundles_submitted ?? 0);
      const landed = Number(pyMetrics.jitoLanded ?? pyMetrics.bundles_landed ?? 0);
      // jito_landed is bundles that successfully landed; submitted ≥ landed
      const bundlesSubmitted = Number(pyMetrics.bundles_submitted ?? pyMetrics.jitoLanded ?? 0);
      const bundlesLanded = Number(pyMetrics.bundles_landed ?? pyMetrics.jitoLanded ?? 0);
      const landedRate = bundlesSubmitted > 0 ? (bundlesLanded / bundlesSubmitted) * 100 : 0;
      const mevSavedSol = Number(pyMetrics.mev_saved_sol ?? pyMetrics.mevSavedSol ?? 0);

      res.json({
        bundlesSubmitted,
        bundlesLanded,
        landedRate,
        mevSavedSol,
        jitoEnabled: jitoEnabled || bundlesSubmitted > 0,
      });
      return;
    }

    // No engine data at all — return zeros but indicate live connectivity status
    res.json({
      bundlesSubmitted: 0,
      bundlesLanded: 0,
      landedRate: 0,
      mevSavedSol: 0,
      jitoEnabled,
      _source: "no-engine",
    });
  } catch {
    res.status(500).json({ error: "Failed to get MEV stats" });
  }
});

// ─── Bot Control ──────────────────────────────────────────────────────────────

router.post("/bot/start", async (_req: Request, res: Response) => {
  try {
    const strategies = ["sniper", "momentum"];
    let succeeded = 0;
    for (const name of strategies) {
      try {
        const result = await fetchPython("/api/strategy/activate", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ strategy_name: name, enabled: true }),
        });
        if (result !== null) succeeded++;
      } catch { /* strategy failed — continue */ }
    }
    res.json({
      success: succeeded > 0,
      message: succeeded > 0
        ? `${succeeded} strategy/strategies activated`
        : "Python engine unreachable — start the python-strategy service first",
    });
  } catch {
    res.json({ success: false, message: "Start request failed" });
  }
});

router.post("/bot/stop", async (_req: Request, res: Response) => {
  try {
    const strategies = ["sniper", "momentum"];
    let succeeded = 0;
    for (const name of strategies) {
      try {
        const result = await fetchPython("/api/strategy/activate", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ strategy_name: name, enabled: false }),
        });
        if (result !== null) succeeded++;
      } catch { /* strategy failed — continue */ }
    }
    res.json({
      success: succeeded > 0,
      message: succeeded > 0
        ? `${succeeded} strategy/strategies deactivated`
        : "Python engine unreachable — engine may already be stopped",
    });
  } catch {
    res.json({ success: false, message: "Stop request failed" });
  }
});

// ─── Helpers ──────────────────────────────────────────────────────────────────

function generateMockTrades(limit: number, strategy?: string) {
  const strategies = strategy ? [strategy] : ["sniper", "momentum", "manual"];
  const statuses = ["Executed", "Pending", "Failed", "Executed", "Executed"];
  const symbols = ["BONK", "WIF", "PNUT", "MOODENG", "POPCAT", "FWOG"];

  return Array.from({ length: Math.min(limit, 20) }, (_, i) => ({
    id: `trade-${i + 1}`,
    mint: `7xKXtg${i}CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU`,
    tokenName: `${symbols[i % symbols.length]} Token`,
    tokenSymbol: symbols[i % symbols.length],
    side: i % 3 === 0 ? "SELL" : "BUY",
    amountSol: parseFloat((Math.random() * 0.5 + 0.01).toFixed(4)),
    price: Math.random() * 0.001,
    status: statuses[i % statuses.length],
    strategy: strategies[i % strategies.length],
    signature: i % 4 !== 0 ? `${i}sig3KLxHvP2mNjRdWsYcAtQfBe7GoZkCuVyi0` : null,
    pnlSol: i % 4 === 0 ? null : parseFloat(((Math.random() - 0.4) * 0.05).toFixed(6)),
    createdAt: new Date(Date.now() - i * 1000 * 60 * 3).toISOString(),
    executedAt: i % 4 !== 0 ? new Date(Date.now() - i * 1000 * 60 * 3 + 2000).toISOString() : null,
  }));
}

export default router;
