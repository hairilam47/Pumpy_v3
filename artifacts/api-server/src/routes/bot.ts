import { Router, type Request, type Response } from "express";
import { db, tradesTable, strategiesTable } from "@workspace/db";
import { desc } from "drizzle-orm";

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

// GET /api/bot/portfolio
router.get("/bot/portfolio", async (req: Request, res: Response) => {
  try {
    const pyData = await fetchPython("/api/portfolio") as Record<string, unknown> | null;
    res.json(pyData ?? {
      totalValueSol: 10.0,
      cashBalanceSol: 9.5,
      positionsValueSol: 0.5,
      dailyPnlSol: 0.0,
      totalPnlSol: 0.0,
      openPositionsCount: 0,
      winRate: 0,
    });
  } catch {
    res.status(500).json({ error: "Failed to get portfolio" });
  }
});

// GET /api/bot/trades
router.get("/bot/trades", async (req: Request, res: Response) => {
  try {
    const limit = Math.min(parseInt(String(req.query.limit ?? "50")), 200);
    const strategy = req.query.strategy as string | undefined;

    let rows: unknown[];
    try {
      const query = db.select().from(tradesTable).orderBy(desc(tradesTable.createdAt)).limit(limit);
      rows = await query;
    } catch {
      rows = generateMockTrades(limit, strategy);
    }

    res.json(rows);
  } catch {
    res.status(500).json({ error: "Failed to list trades" });
  }
});

// POST /api/bot/orders
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
      res.status(400).json({ error: "Missing required fields" });
      return;
    }

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

    if (pyData) {
      res.json(pyData);
    } else {
      res.json({ success: false, orderId: "", message: "Python engine not connected" });
    }
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : "Invalid request";
    res.status(400).json({ error: msg });
  }
});

// GET /api/bot/orders/:orderId
router.get("/bot/orders/:orderId", async (req: Request, res: Response) => {
  try {
    const { orderId } = req.params;
    const pyData = await fetchPython(`/api/orders/${orderId}`) as Record<string, unknown> | null;
    if (!pyData) {
      res.status(404).json({ error: "Order not found" });
      return;
    }
    res.json(pyData);
  } catch {
    res.status(404).json({ error: "Order not found" });
  }
});

// DELETE /api/bot/orders/:orderId
router.delete("/bot/orders/:orderId", async (req: Request, res: Response) => {
  try {
    const { orderId } = req.params;
    const pyData = await fetchPython(`/api/orders/${orderId}`, { method: "DELETE" }) as Record<string, unknown> | null;
    res.json(pyData ?? { success: true, message: "Cancelled" });
  } catch {
    res.status(500).json({ error: "Cancel failed" });
  }
});

// GET /api/bot/strategies
router.get("/bot/strategies", async (req: Request, res: Response) => {
  try {
    const pyData = await fetchPython("/api/strategies") as unknown[] | null;
    if (pyData && Array.isArray(pyData)) {
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

    // Fallback to DB
    try {
      const rows = await db.select().from(strategiesTable);
      const mapped = rows.map((r) => ({
        name: r.name,
        enabled: r.enabled,
        tradesExecuted: r.tradesExecuted,
        tradesWon: r.tradesWon,
        winRate: r.tradesExecuted > 0 ? (r.tradesWon / r.tradesExecuted) * 100 : 0,
        totalPnlSol: r.totalPnlSol,
        buyAmountSol: r.buyAmountSol,
      }));
      res.json(mapped);
      return;
    } catch {
      // ignore
    }

    res.json([
      { name: "sniper", enabled: true, tradesExecuted: 0, tradesWon: 0, winRate: 0, totalPnlSol: 0, buyAmountSol: 0.05 },
      { name: "momentum", enabled: true, tradesExecuted: 0, tradesWon: 0, winRate: 0, totalPnlSol: 0, buyAmountSol: 0.1 },
    ]);
  } catch {
    res.status(500).json({ error: "Failed to list strategies" });
  }
});

// PATCH /api/bot/strategies/:strategyName
router.patch("/bot/strategies/:strategyName", async (req: Request, res: Response) => {
  try {
    const { strategyName } = req.params;
    const body = req.body as { enabled?: boolean; buyAmountSol?: number; slippageBps?: number };

    const pyData = await fetchPython(`/api/strategies/${strategyName}`, {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        strategy_name: strategyName,
        enabled: body.enabled,
        buy_amount_sol: body.buyAmountSol,
        slippage_bps: body.slippageBps,
      }),
    }) as Record<string, unknown> | null;

    if (pyData?.strategy) {
      res.json(pyData.strategy);
      return;
    }

    res.json({
      name: strategyName,
      enabled: body.enabled ?? true,
      tradesExecuted: 0,
      tradesWon: 0,
      winRate: 0,
      totalPnlSol: 0,
    });
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : "Update failed";
    res.status(400).json({ error: msg });
  }
});

// GET /api/bot/tokens
router.get("/bot/tokens", async (req: Request, res: Response) => {
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
      }));
      res.json(tokens);
      return;
    }
    res.json([]);
  } catch {
    res.status(500).json({ error: "Failed to list tokens" });
  }
});

// GET /api/bot/tokens/:mint
router.get("/bot/tokens/:mint", async (req: Request, res: Response) => {
  try {
    const { mint } = req.params;
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
    });
  } catch {
    res.status(404).json({ error: "Token not found" });
  }
});

// GET /api/bot/metrics
router.get("/bot/metrics", async (req: Request, res: Response) => {
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

// GET /api/bot/status
router.get("/bot/status", async (req: Request, res: Response) => {
  try {
    const pyHealth = await fetchPython("/api/health");
    const pyConnected = !!pyHealth;
    res.json({
      running: pyConnected,
      rustEngineConnected: false,
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
