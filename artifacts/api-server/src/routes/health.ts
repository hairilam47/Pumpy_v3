import { Router, type IRouter, type Request, type Response } from "express";
import { HealthCheckResponse } from "@workspace/api-zod";
import { pool } from "@workspace/db";
import type { PoolClient } from "pg";
import { getPythonStatus } from "../lib/python-status";

const router: IRouter = Router();

const CONNECT_TIMEOUT_MS = 3000;
const QUERY_TIMEOUT_MS = 3000;

router.get("/healthz", (_req, res) => {
  const base = HealthCheckResponse.parse({ status: "ok" });
  res.json({ ...base, python: getPythonStatus() });
});

router.get("/readiness", async (_req: Request, res: Response) => {
  let client: PoolClient | null = null;
  let connectTimedOut = false;
  const connectPromise = pool.connect();

  try {
    client = await Promise.race([
      connectPromise,
      new Promise<never>((_, reject) =>
        setTimeout(() => {
          connectTimedOut = true;
          reject(new Error(`DB pool did not yield a connection within ${CONNECT_TIMEOUT_MS}ms`));
        }, CONNECT_TIMEOUT_MS)
      ),
    ]);

    await client.query(`SET LOCAL statement_timeout = ${QUERY_TIMEOUT_MS}`);
    await client.query("SELECT 1");

    res.json({ status: "ok", db: "reachable" });
  } catch (err) {
    const message = err instanceof Error ? err.message : "DB check failed";
    if (!res.headersSent) {
      res.status(503).json({ status: "unavailable", error: message });
    }
  } finally {
    if (client) {
      client.release();
    } else if (connectTimedOut) {
      connectPromise.then((c) => c.release()).catch(() => {});
    }
  }
});

export default router;
