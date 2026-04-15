import { Router, type Request, type Response, type NextFunction } from "express";
import { db, systemConfigTable } from "@workspace/db";

const router = Router();

/**
 * Operator-only guard — requires a valid ADMIN_API_KEY header.
 * If ADMIN_API_KEY env var is not set, all admin endpoints return 503 to prevent
 * accidental exposure of operator config in environments without the secret.
 */
function requireAdminKey(req: Request, res: Response, next: NextFunction): void {
  const adminKey = process.env.ADMIN_API_KEY;
  if (!adminKey) {
    res.status(503).json({ error: "Admin endpoints are disabled: ADMIN_API_KEY is not configured" });
    return;
  }
  const provided = req.headers["x-admin-key"];
  if (!provided || provided !== adminKey) {
    res.status(401).json({ error: "Unauthorized: valid X-Admin-Key header required" });
    return;
  }
  next();
}

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
