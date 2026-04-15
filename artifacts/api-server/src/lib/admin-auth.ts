import type { Request, Response, NextFunction } from "express";

/**
 * Operator-only auth guard.
 * Requires the X-Admin-Key header to match the ADMIN_API_KEY environment variable.
 * If ADMIN_API_KEY is not configured, all protected endpoints return 503 (fail-closed).
 */
export function requireAdminKey(req: Request, res: Response, next: NextFunction): void {
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
