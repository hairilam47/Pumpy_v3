import { Router } from "express";
import { eq } from "drizzle-orm";
import { db, walletRegistryTable, walletConfigTable } from "@workspace/db";
import { requireAdminKey } from "../lib/admin-auth.js";

const router = Router();

const PYTHON_API = process.env.PYTHON_API_URL ?? "http://localhost:8001";

const VALID_PRESETS = new Set(["conservative", "balanced", "aggressive"]);
const VALID_STATUSES = new Set(["enabled", "paused", "halted"]);

const PRESET_PARAMS: Record<string, {
  risk_per_trade_sol: number;
  stop_loss_pct: number;
  take_profit_pct: number;
  max_positions: number;
}> = {
  conservative: { risk_per_trade_sol: 0.05, stop_loss_pct: 5, take_profit_pct: 20, max_positions: 2 },
  balanced:     { risk_per_trade_sol: 0.15, stop_loss_pct: 10, take_profit_pct: 50, max_positions: 5 },
  aggressive:   { risk_per_trade_sol: 0.5,  stop_loss_pct: 20, take_profit_pct: 100, max_positions: 10 },
};

// ─── GET /api/wallets ─────────────────────────────────────────────────────────

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

// ─── GET /api/wallets/:id/config ──────────────────────────────────────────────

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

// ─── PUT /api/wallets/:id/config — admin-gated ────────────────────────────────

router.put("/wallets/:id/config", requireAdminKey, async (req, res) => {
  const { id } = req.params;
  const body = req.body as Record<string, unknown>;

  const configUpdate: Partial<{
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
    configUpdate.riskPerTradeSol = v;
  }

  if (body.daily_loss_limit_sol !== undefined) {
    const v = Number(body.daily_loss_limit_sol);
    if (isNaN(v) || v <= 0) {
      res.status(400).json({ error: "daily_loss_limit_sol must be a positive number" });
      return;
    }
    configUpdate.dailyLossLimitSol = v;
  }

  if (body.strategy_preset !== undefined) {
    if (!VALID_PRESETS.has(String(body.strategy_preset))) {
      res.status(400).json({ error: "strategy_preset must be conservative, balanced, or aggressive" });
      return;
    }
    configUpdate.strategyPreset = String(body.strategy_preset);
  }

  let newStatus: string | undefined;
  if (body.status !== undefined) {
    if (!VALID_STATUSES.has(String(body.status))) {
      res.status(400).json({ error: "status must be enabled, paused, or halted" });
      return;
    }
    newStatus = String(body.status);
    configUpdate.status = newStatus;
  }

  try {
    const rows = await db.transaction(async (tx) => {
      const updated = await tx
        .update(walletConfigTable)
        .set(configUpdate)
        .where(eq(walletConfigTable.walletId, id!))
        .returning();

      if (updated.length === 0) {
        return updated;
      }

      if (newStatus !== undefined) {
        await tx
          .update(walletRegistryTable)
          .set({ status: newStatus })
          .where(eq(walletRegistryTable.walletId, id!));
      }

      return updated;
    });

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

// ─── PUT /api/strategy/preset — admin-gated ──────────────────────────────────
// Changes live risk posture (position size, stop-loss, take-profit) for a
// wallet. Admin key is required to prevent unauthorized strategy mutations.

router.put("/strategy/preset", requireAdminKey, async (req, res) => {
  const body = req.body as { preset?: string; wallet_id?: string };
  const preset = body.preset;
  const walletId = body.wallet_id ?? "wallet_001";

  if (!preset || !VALID_PRESETS.has(preset)) {
    res.status(400).json({ error: "preset must be conservative, balanced, or aggressive" });
    return;
  }

  const params = PRESET_PARAMS[preset]!;

  try {
    // Ensure wallet_config row exists (upsert), then apply the preset params.
    const updated = await db
      .update(walletConfigTable)
      .set({
        strategyPreset: preset,
        riskPerTradeSol: params.risk_per_trade_sol,
        updatedAt: new Date(),
      })
      .where(eq(walletConfigTable.walletId, walletId))
      .returning();

    if (updated.length === 0) {
      res.status(404).json({ error: `Wallet config for '${walletId}' not found` });
      return;
    }

    // Notify Python engine for in-memory update (best-effort).
    try {
      const pyResp = await fetch(`${PYTHON_API}/api/strategy/preset`, {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ preset, wallet_id: walletId }),
        signal: AbortSignal.timeout(3000),
      });
      if (!pyResp.ok) {
        console.warn(`PUT /strategy/preset: Python engine returned ${pyResp.status}; DB updated, in-memory update skipped`);
      }
    } catch (pyErr) {
      console.warn("PUT /strategy/preset: Python engine unreachable; DB updated, in-memory update skipped", pyErr);
    }

    res.json({ ok: true, preset, walletId, params });
  } catch (err) {
    console.error("PUT /strategy/preset error:", err);
    res.status(500).json({ error: "Failed to save preset" });
  }
});

// ─── GET /api/strategy/preset — preset definitions (read-only) ───────────────

router.get("/strategy/preset", (_req, res) => {
  res.json(PRESET_PARAMS);
});

// ─── POST /api/wallets/:id/pause — admin-gated ───────────────────────────────
// Pausing halts live order execution; admin key is required to prevent
// unauthenticated callers from triggering an operational DoS on the wallet.

router.post("/wallets/:id/pause", requireAdminKey, async (req, res) => {
  const { id } = req.params;
  try {
    await db.transaction(async (tx) => {
      await tx
        .update(walletRegistryTable)
        .set({ status: "paused" })
        .where(eq(walletRegistryTable.walletId, id!));

      await tx
        .update(walletConfigTable)
        .set({ status: "paused", updatedAt: new Date() })
        .where(eq(walletConfigTable.walletId, id!));
    });

    res.json({ ok: true, walletId: id, status: "paused" });
  } catch (err) {
    console.error(`POST /wallets/${id}/pause error:`, err);
    res.status(500).json({ error: "Failed to pause wallet" });
  }
});

// ─── POST /api/wallets/:id/resume — admin-gated ──────────────────────────────
// Resuming a paused wallet re-enables live trading; admin key is required to
// prevent unauthenticated callers from lifting a safety pause.

router.post("/wallets/:id/resume", requireAdminKey, async (req, res) => {
  const { id } = req.params;
  try {
    await db.transaction(async (tx) => {
      await tx
        .update(walletRegistryTable)
        .set({ status: "enabled" })
        .where(eq(walletRegistryTable.walletId, id!));

      await tx
        .update(walletConfigTable)
        .set({ status: "enabled", updatedAt: new Date() })
        .where(eq(walletConfigTable.walletId, id!));
    });

    res.json({ ok: true, walletId: id, status: "enabled" });
  } catch (err) {
    console.error(`POST /wallets/${id}/resume error:`, err);
    res.status(500).json({ error: "Failed to resume wallet" });
  }
});

// ─── POST /api/wallets/:id/halt — admin-gated ────────────────────────────────
// Halt is a permanent (destructive) action; admin key is required.

router.post("/wallets/:id/halt", requireAdminKey, async (req, res) => {
  const { id } = req.params;
  const { confirm } = req.body as { confirm?: boolean };
  if (!confirm) {
    res.status(400).json({ error: "Halting a wallet requires { confirm: true } in the request body" });
    return;
  }
  try {
    await db.transaction(async (tx) => {
      await tx
        .update(walletRegistryTable)
        .set({ status: "halted" })
        .where(eq(walletRegistryTable.walletId, id!));

      await tx
        .update(walletConfigTable)
        .set({ status: "halted", updatedAt: new Date() })
        .where(eq(walletConfigTable.walletId, id!));
    });

    res.json({ ok: true, walletId: id, status: "halted" });
  } catch (err) {
    console.error(`POST /wallets/${id}/halt error:`, err);
    res.status(500).json({ error: "Failed to halt wallet" });
  }
});

export default router;
