import { Router, type Request, type Response } from "express";
import { db, tokenMetricsTable } from "@workspace/db";
import { gte, desc, eq, and, inArray } from "drizzle-orm";

const router = Router();

// POST /api/token-metrics  →  batch record price snapshots for tokens
router.post("/token-metrics", async (req: Request, res: Response) => {
  try {
    const body = req.body as {
      snapshots?: Array<{
        mint: string;
        price: number;
        liquidity_sol?: number;
        market_cap_sol?: number;
        volume_24h_sol?: number;
        holder_count?: number;
        bonding_curve_progress?: number;
      }>;
    };

    if (!Array.isArray(body.snapshots) || body.snapshots.length === 0) {
      res.status(400).json({ error: "snapshots array is required" });
      return;
    }

    const rows = body.snapshots.map((s) => ({
      mint: s.mint,
      price: s.price,
      liquiditySol: s.liquidity_sol ?? null,
      marketCapSol: s.market_cap_sol ?? null,
      volume24hSol: s.volume_24h_sol ?? null,
      holderCount: s.holder_count ?? null,
      bondingCurveProgress: s.bonding_curve_progress ?? null,
    }));

    await db.insert(tokenMetricsTable).values(rows);
    res.json({ inserted: rows.length });
  } catch (err) {
    res.status(500).json({ error: "Failed to record token metrics" });
  }
});

// GET /api/token-metrics?mint=...&days=7  →  query historical snapshots
// `mint` may be a single string or an array (repeated query param)
router.get("/token-metrics", async (req: Request, res: Response) => {
  try {
    const days = Math.max(1, Math.min(30, Number(req.query.days ?? 7)));
    const limit = Math.max(1, Math.min(5000, Number(req.query.limit ?? 1000)));
    const cutoff = new Date(Date.now() - days * 24 * 60 * 60 * 1000);

    // Normalise `mint` to an array of strings (handles ?mint=A&mint=B or ?mint=A)
    const rawMint = req.query.mint;
    const mints: string[] = Array.isArray(rawMint)
      ? (rawMint as string[])
      : typeof rawMint === "string"
        ? [rawMint]
        : [];

    let conditions;
    if (mints.length === 1) {
      conditions = and(gte(tokenMetricsTable.recordedAt, cutoff), eq(tokenMetricsTable.mint, mints[0]!));
    } else if (mints.length > 1) {
      conditions = and(gte(tokenMetricsTable.recordedAt, cutoff), inArray(tokenMetricsTable.mint, mints));
    } else {
      conditions = gte(tokenMetricsTable.recordedAt, cutoff);
    }

    const rows = await db
      .select()
      .from(tokenMetricsTable)
      .where(conditions)
      .orderBy(desc(tokenMetricsTable.recordedAt))
      .limit(limit);

    res.json(rows);
  } catch (err) {
    res.status(500).json({ error: "Failed to query token metrics" });
  }
});

export default router;
