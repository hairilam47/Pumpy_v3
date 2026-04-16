import { Router, type Request, type Response } from "express";
import { eq, and, desc } from "drizzle-orm";
import { db, tradesTable } from "@workspace/db";

const router = Router();

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

// ─── GET /api/bot/trades ─────────────────────────────────────────────────────
// Supports optional query filters:
//   ?clientOrderId=<uuid>  — exact match via trades_client_order_id_idx
//   ?strategy=<name>       — filter by strategy name
//   ?limit=<n>             — max rows (default 50, max 200)

router.get("/bot/trades", async (req: Request, res: Response) => {
  try {
    const limit = Math.min(parseInt(String(req.query.limit ?? "50")), 200);
    const strategy = req.query.strategy as string | undefined;
    const clientOrderId = req.query.clientOrderId as string | undefined;

    if (clientOrderId && !UUID_RE.test(clientOrderId)) {
      res.status(400).json({ error: "clientOrderId must be a valid UUID" });
      return;
    }

    const conditions = [];
    if (clientOrderId) conditions.push(eq(tradesTable.clientOrderId, clientOrderId));
    if (strategy) conditions.push(eq(tradesTable.strategy, strategy));

    const rows = await db
      .select()
      .from(tradesTable)
      .where(conditions.length > 0 ? and(...conditions) : undefined)
      .orderBy(desc(tradesTable.createdAt))
      .limit(limit);

    res.json(rows);
  } catch (err) {
    res.status(500).json({ error: "Failed to list trades" });
  }
});

// ─── GET /api/bot/trades/:clientOrderId ──────────────────────────────────────
// Look up a single trade by its client_order_id.
// Uses the trades_client_order_id_idx partial index for efficient lookup.

router.get("/bot/trades/:clientOrderId", async (req: Request, res: Response) => {
  const clientOrderId = req.params["clientOrderId"] as string;

  if (!UUID_RE.test(clientOrderId)) {
    res.status(400).json({ error: "clientOrderId must be a valid UUID" });
    return;
  }

  try {
    const rows = await db
      .select()
      .from(tradesTable)
      .where(eq(tradesTable.clientOrderId, clientOrderId as string))
      .limit(1);

    if (rows.length === 0) {
      res.status(404).json({ error: "Trade not found" });
      return;
    }

    res.json(rows[0]);
  } catch (err) {
    res.status(500).json({ error: "Failed to look up trade" });
  }
});

export default router;
