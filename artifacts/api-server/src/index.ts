import { createServer } from "http";
import { WebSocketServer, type WebSocket } from "ws";
import { eq } from "drizzle-orm";
import app from "./app";
import { logger } from "./lib/logger";
import { grpcBot } from "./lib/grpc-client";
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

// WebSocket server for /api/bot/stream  (StreamOrders gRPC bridge)
const wss = new WebSocketServer({ noServer: true });

wss.on("connection", (ws: WebSocket, orderIds: string[]) => {
  logger.info({ orderIds }, "WS client connected to /api/bot/stream");

  let cancelGrpc: (() => void) | null = null;

  function startStream(ids: string[]) {
    try {
      cancelGrpc = grpcBot.streamOrders(
        ids,
        async (update) => {
          if (ws.readyState !== ws.OPEN) return;

          // Base DTO normalized from gRPC snake_case to camelCase
          const normalized: Record<string, unknown> = {
            id: update.order_id,
            mint: update.token_mint ?? "",
            status: update.status,
            signature: update.signature,
            error: update.error,
            executedAt: update.executed_at,
            executedPrice: update.executed_price,
            amountSol: update.executed_amount != null
              ? Number(update.executed_amount) / 1e9
              : undefined,
            createdAt: new Date().toISOString(),
          };

          // Enrich with side, symbol, strategy, pnlSol from trade DB record
          try {
            const rows = await db
              .select()
              .from(tradesTable)
              .where(eq(tradesTable.id, update.order_id))
              .limit(1);
            if (rows.length > 0) {
              const row = rows[0]!;
              normalized.side = row.side;
              normalized.tokenSymbol = row.tokenSymbol;
              normalized.tokenName = row.tokenName;
              normalized.strategy = row.strategy;
              normalized.pnlSol = row.pnlSol;
              if (normalized.amountSol == null) normalized.amountSol = row.amountSol;
            }
          } catch {
            // DB not available — continue with base fields
          }

          if (ws.readyState === ws.OPEN) {
            ws.send(JSON.stringify(normalized));
          }
        },
        (err) => {
          if (err) {
            logger.warn({ err }, "gRPC stream ended with error, retrying in 5s");
            setTimeout(() => {
              if (ws.readyState === ws.OPEN) startStream(ids);
            }, 5_000);
          }
        },
      );
    } catch (err) {
      logger.warn({ err }, "gRPC streamOrders unavailable — Rust engine not running");
      // Retry after 10s — don't crash the server
      setTimeout(() => {
        if (ws.readyState === ws.OPEN) startStream(ids);
      }, 10_000);
    }
  }

  startStream(orderIds);

  ws.on("message", (data) => {
    try {
      const msg = JSON.parse(data.toString()) as { order_ids?: string[] };
      if (Array.isArray(msg.order_ids)) {
        cancelGrpc?.();
        startStream(msg.order_ids);
      }
    } catch { /* ignore parse errors */ }
  });

  ws.on("close", () => {
    cancelGrpc?.();
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

httpServer.listen(port, () => {
  logger.info({ port }, "Server listening (HTTP + WebSocket)");
});
