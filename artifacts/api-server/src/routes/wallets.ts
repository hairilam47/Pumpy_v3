import { Router } from "express";
import { eq } from "drizzle-orm";
import { db, walletRegistryTable, walletConfigTable } from "@workspace/db";

const router = Router();

const VALID_PRESETS = new Set(["conservative", "balanced", "aggressive"]);
const VALID_STATUSES = new Set(["enabled", "paused", "halted"]);

router.get("/wallets", async (_req, res) => {
  try {
    const rows = await db
      .select({
        walletId: walletRegistryTable.walletId,
        status: walletRegistryTable.status,
        ownerPubkey: walletRegistryTable.ownerPubkey,
        lastActiveAt: walletRegistryTable.lastActiveAt,
        createdAt: walletRegistryTable.createdAt,
      })
      .from(walletRegistryTable)
      .orderBy(walletRegistryTable.createdAt);

    res.json(rows);
  } catch (err) {
    console.error("GET /wallets error:", err);
    res.status(500).json({ error: "Failed to list wallets" });
  }
});

router.get("/wallets/:id/config", async (req, res) => {
  const { id } = req.params;
  try {
    const rows = await db
      .select()
      .from(walletConfigTable)
      .where(eq(walletConfigTable.walletId, id!))
      .limit(1);

    if (rows.length === 0) {
      res.status(404).json({ error: "Wallet config not found" });
      return;
    }
    res.json(rows[0]);
  } catch (err) {
    console.error(`GET /wallets/${id}/config error:`, err);
    res.status(500).json({ error: "Failed to load wallet config" });
  }
});

router.put("/wallets/:id/config", async (req, res) => {
  const { id } = req.params;
  const body = req.body as Record<string, unknown>;

  const update: Partial<{
    riskPerTradeSol: number;
    dailyLossLimitSol: number;
    strategyPreset: string;
    status: string;
    updatedAt: Date;
  }> = { updatedAt: new Date() };

  if (body.risk_per_trade_sol !== undefined) {
    const v = Number(body.risk_per_trade_sol);
    if (isNaN(v) || v <= 0) {
      res.status(400).json({ error: "risk_per_trade_sol must be a positive number" });
      return;
    }
    update.riskPerTradeSol = v;
  }

  if (body.daily_loss_limit_sol !== undefined) {
    const v = Number(body.daily_loss_limit_sol);
    if (isNaN(v) || v <= 0) {
      res.status(400).json({ error: "daily_loss_limit_sol must be a positive number" });
      return;
    }
    update.dailyLossLimitSol = v;
  }

  if (body.strategy_preset !== undefined) {
    if (!VALID_PRESETS.has(String(body.strategy_preset))) {
      res.status(400).json({ error: "strategy_preset must be conservative, balanced, or aggressive" });
      return;
    }
    update.strategyPreset = String(body.strategy_preset);
  }

  if (body.status !== undefined) {
    if (!VALID_STATUSES.has(String(body.status))) {
      res.status(400).json({ error: "status must be enabled, paused, or halted" });
      return;
    }
    update.status = String(body.status);
  }

  try {
    const rows = await db
      .update(walletConfigTable)
      .set(update)
      .where(eq(walletConfigTable.walletId, id!))
      .returning();

    if (rows.length === 0) {
      res.status(404).json({ error: "Wallet config not found" });
      return;
    }
    res.json(rows[0]);
  } catch (err) {
    console.error(`PUT /wallets/${id}/config error:`, err);
    res.status(500).json({ error: "Failed to update wallet config" });
  }
});

router.post("/wallets/:id/pause", async (req, res) => {
  const { id } = req.params;
  try {
    await db
      .update(walletRegistryTable)
      .set({ status: "paused" })
      .where(eq(walletRegistryTable.walletId, id!));

    const rows = await db
      .update(walletConfigTable)
      .set({ status: "paused", updatedAt: new Date() })
      .where(eq(walletConfigTable.walletId, id!))
      .returning();

    if (rows.length === 0) {
      res.status(404).json({ error: "Wallet not found" });
      return;
    }
    res.json({ ok: true, walletId: id, status: "paused" });
  } catch (err) {
    console.error(`POST /wallets/${id}/pause error:`, err);
    res.status(500).json({ error: "Failed to pause wallet" });
  }
});

router.post("/wallets/:id/resume", async (req, res) => {
  const { id } = req.params;
  try {
    await db
      .update(walletRegistryTable)
      .set({ status: "enabled" })
      .where(eq(walletRegistryTable.walletId, id!));

    const rows = await db
      .update(walletConfigTable)
      .set({ status: "enabled", updatedAt: new Date() })
      .where(eq(walletConfigTable.walletId, id!))
      .returning();

    if (rows.length === 0) {
      res.status(404).json({ error: "Wallet not found" });
      return;
    }
    res.json({ ok: true, walletId: id, status: "enabled" });
  } catch (err) {
    console.error(`POST /wallets/${id}/resume error:`, err);
    res.status(500).json({ error: "Failed to resume wallet" });
  }
});

router.post("/wallets/:id/halt", async (req, res) => {
  const { id } = req.params;
  const { confirm } = req.body as { confirm?: boolean };
  if (!confirm) {
    res.status(400).json({ error: "Halting a wallet requires { confirm: true } in the request body" });
    return;
  }
  try {
    await db
      .update(walletRegistryTable)
      .set({ status: "halted" })
      .where(eq(walletRegistryTable.walletId, id!));

    const rows = await db
      .update(walletConfigTable)
      .set({ status: "halted", updatedAt: new Date() })
      .where(eq(walletConfigTable.walletId, id!))
      .returning();

    if (rows.length === 0) {
      res.status(404).json({ error: "Wallet not found" });
      return;
    }
    res.json({ ok: true, walletId: id, status: "halted" });
  } catch (err) {
    console.error(`POST /wallets/${id}/halt error:`, err);
    res.status(500).json({ error: "Failed to halt wallet" });
  }
});

export default router;
