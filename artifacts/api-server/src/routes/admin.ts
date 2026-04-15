import { Router } from "express";
import { db, systemConfigTable } from "@workspace/db";
import { requireAdminKey } from "../lib/admin-auth.js";

const router = Router();

router.get("/admin/system-config", requireAdminKey, async (_req, res) => {
  try {
    const rows = await db.select().from(systemConfigTable).orderBy(systemConfigTable.key);
    res.json(rows);
  } catch (err) {
    console.error("GET /admin/system-config error:", err);
    res.status(500).json({ error: "Failed to load system config" });
  }
});

export default router;
