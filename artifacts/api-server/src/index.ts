import { createServer } from "http";
import { EventEmitter } from "events";
import { WebSocketServer, type WebSocket } from "ws";
import type * as grpc from "@grpc/grpc-js";
import { eq } from "drizzle-orm";
import app from "./app";
import { logger } from "./lib/logger";
import { grpcBot, type OrderUpdate } from "./lib/grpc-client";
import { db, tradesTable } from "@workspace/db";

const rawPort = process.env["PORT"];

if (!rawPort) {
  throw new Error(
    "PORT environment variable is required but was not provided.",
  );
}

const port = Number(rawPort);

if (Number.isNaN(port) || port <= 0) {
  throw new Error(`Invalid PORT value: "${rawPort}"`);
}

const httpServer = createServer(app);

// ── Singleton gRPC stream with fan-out (Task #27) ────────────────────────────
// A single StreamOrders call feeds all WebSocket clients via an EventEmitter,
// instead of one gRPC stream per WS client.

interface EnrichedUpdate extends OrderUpdate {
  side?: string;
  tokenSymbol?: string;
  tokenName?: string;
  strategy?: string;
  pnlSol?: number | null;
  amountSol?: number;
  createdAt?: string;
}

const orderEmitter = new EventEmitter();
orderEmitter.setMaxListeners(2000);

let _singletonActive = false;
let _singletonCancelFn: (() => void) | null = null;

async function enrichUpdate(update: OrderUpdate): Promise<EnrichedUpdate> {
  const normalized: EnrichedUpdate = {
    ...update,
    amountSol: update.executed_amount != null
      ? Number(update.executed_amount) / 1e9
      : undefined,
    createdAt: new Date().toISOString(),
  };

  try {
    const rows = await db
      .select()
      .from(tradesTable)
      .where(eq(tradesTable.id, update.order_id))
      .limit(1);
    if (rows.length > 0) {
      const row = rows[0]!;
      normalized.side = row.side;
      normalized.tokenSymbol = row.tokenSymbol ?? undefined;
      normalized.tokenName = row.tokenName ?? undefined;
      normalized.strategy = row.strategy;
      normalized.pnlSol = row.pnlSol;
      if (normalized.amountSol == null) normalized.amountSol = row.amountSol;
    }
  } catch {
    // DB not available — continue with base fields
  }

  return normalized;
}

function startSingletonStream() {
  if (_singletonActive) return;
  _singletonActive = true;

  logger.info("Starting singleton gRPC StreamOrders");

  try {
    _singletonCancelFn = grpcBot.streamOrders(
      [],  // empty = all orders
      async (update: OrderUpdate) => {
        const enriched = await enrichUpdate(update);
        orderEmitter.emit("order", enriched);
      },
      (err?: grpc.ServiceError) => {
        _singletonActive = false;
        _singletonCancelFn = null;
        if (err) {
          logger.warn({ err }, "Singleton gRPC stream ended with error, reconnecting in 5s");
          setTimeout(startSingletonStream, 5_000);
        } else {
          logger.info("Singleton gRPC stream ended cleanly, reconnecting in 3s");
          setTimeout(startSingletonStream, 3_000);
        }
      },
    );
  } catch (err) {
    _singletonActive = false;
    logger.warn({ err }, "gRPC streamOrders unavailable — Rust engine not running, retrying in 10s");
    setTimeout(startSingletonStream, 10_000);
  }
}

// ── WebSocket server ──────────────────────────────────────────────────────────
const wss = new WebSocketServer({ noServer: true });

wss.on("connection", (ws: WebSocket, subscribedOrderIds: string[]) => {
  logger.info({ subscribedOrderIds }, "WS client connected to /api/bot/stream");

  // Filter: empty = receive all; non-empty = only listed order_ids
  let filterIds = new Set<string>(subscribedOrderIds);

  const onOrder = (update: EnrichedUpdate) => {
    if (ws.readyState !== ws.OPEN) return;
    if (filterIds.size > 0 && !filterIds.has(update.order_id)) return;
    ws.send(JSON.stringify(update));
  };

  orderEmitter.on("order", onOrder);

  ws.on("message", (data) => {
    try {
      const msg = JSON.parse(data.toString()) as { order_ids?: string[] };
      if (Array.isArray(msg.order_ids)) {
        filterIds = new Set<string>(msg.order_ids);
        logger.debug({ filterIds: [...filterIds] }, "WS client updated order filter");
      }
    } catch { /* ignore parse errors */ }
  });

  ws.on("close", () => {
    orderEmitter.off("order", onOrder);
    logger.info("WS client disconnected from /api/bot/stream");
  });
});

// Upgrade HTTP connections to WebSocket only for /api/bot/stream
httpServer.on("upgrade", (req, socket, head) => {
  const url = req.url ?? "";
  if (url.startsWith("/api/bot/stream")) {
    const params = new URL(url, "http://localhost");
    const orderIds = params.searchParams.getAll("order_id");

    wss.handleUpgrade(req, socket, head, (ws) => {
      wss.emit("connection", ws, orderIds);
    });
  } else {
    socket.destroy();
  }
});

// Start the singleton gRPC stream once at server boot
startSingletonStream();

httpServer.listen(port, () => {
  logger.info({ port }, "Server listening (HTTP + WebSocket)");
});
