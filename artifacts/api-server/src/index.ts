import { createServer } from "http";
import { spawn } from "child_process";
import path from "path";
import { existsSync } from "fs";
import { fileURLToPath } from "url";
import { EventEmitter } from "events";
import { WebSocketServer, type WebSocket } from "ws";
import type * as grpc from "@grpc/grpc-js";
import { eq } from "drizzle-orm";
import app from "./app";
import { logger } from "./lib/logger";
import { grpcBot, type OrderUpdate } from "./lib/grpc-client";
import { setPythonStatus } from "./lib/python-status";
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

interface LiveTradePayload {
  id: string;
  mint: string;
  status: string;
  signature?: string;
  executedAt?: string;
  price?: number;
  amountSol?: number;
  side?: string;
  tokenSymbol?: string;
  tokenName?: string;
  strategy?: string;
  pnlSol?: number | null;
  createdAt: string;
  traceId?: string | null;
}

const orderEmitter = new EventEmitter();
orderEmitter.setMaxListeners(2000);

let _singletonActive = false;
let _singletonCancelFn: (() => void) | null = null;

async function enrichUpdate(update: OrderUpdate): Promise<LiveTradePayload> {
  const payload: LiveTradePayload = {
    id: update.order_id,
    mint: update.token_mint ?? "",
    status: update.status,
    signature: update.signature,
    executedAt: update.executed_at,
    price: update.executed_price,
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
      payload.side = row.side;
      payload.tokenSymbol = row.tokenSymbol ?? undefined;
      payload.tokenName = row.tokenName ?? undefined;
      payload.strategy = row.strategy;
      payload.pnlSol = row.pnlSol;
      payload.traceId = row.traceId ?? null;
      if (payload.amountSol == null) payload.amountSol = row.amountSol;
    }
  } catch {
    // DB not available — continue with base fields
  }

  logger.info(
    {
      event: "order_relay",
      order_id: update.order_id,
      status: update.status,
      trace_id: payload.traceId ?? "no-trace",
    },
    "Relaying order event to WebSocket clients",
  );

  return payload;
}

// ── Exponential backoff for singleton reconnect ───────────────────────────────
const BACKOFF_BASE_MS = 1_000;
const BACKOFF_MAX_MS = 60_000;
const BACKOFF_MULTIPLIER = 2;
const BACKOFF_JITTER = 0.25;
const BACKOFF_RESET_AFTER_MS = 30_000;

let _backoffAttempt = 0;
let _streamHealthTimer: ReturnType<typeof setTimeout> | null = null;
let _reconnectTimer: ReturnType<typeof setTimeout> | null = null;

function computeBackoffDelay(): number {
  const exp = Math.min(_backoffAttempt, 10);
  const base = Math.min(BACKOFF_BASE_MS * Math.pow(BACKOFF_MULTIPLIER, exp), BACKOFF_MAX_MS);
  const jitter = base * BACKOFF_JITTER * (Math.random() * 2 - 1);
  return Math.round(base + jitter);
}

function resetBackoff() {
  _backoffAttempt = 0;
  if (_streamHealthTimer) {
    clearTimeout(_streamHealthTimer);
    _streamHealthTimer = null;
  }
}

function scheduleReconnect(hadError: boolean) {
  // Guard against duplicate timers when gRPC emits both error and end events.
  if (_reconnectTimer !== null) return;

  _backoffAttempt += 1;
  const delay = computeBackoffDelay();
  logger.warn(
    { attempt: _backoffAttempt, delayMs: delay, hadError },
    "Singleton gRPC stream ended — scheduling reconnect with exponential backoff",
  );
  _reconnectTimer = setTimeout(() => {
    _reconnectTimer = null;
    startSingletonStream();
  }, delay);
}

function startSingletonStream() {
  if (_singletonActive) return;
  _singletonActive = true;

  logger.info({ attempt: _backoffAttempt }, "Starting singleton gRPC StreamOrders");

  try {
    _singletonCancelFn = grpcBot.streamOrders(
      [],
      async (update: OrderUpdate) => {
        // Schedule a backoff reset only after the stream has been healthy for a
        // sustained period. If the timer is already pending, leave it running.
        if (_streamHealthTimer === null) {
          _streamHealthTimer = setTimeout(resetBackoff, BACKOFF_RESET_AFTER_MS);
        }
        const payload = await enrichUpdate(update);
        orderEmitter.emit("order", payload);
      },
      (err?: grpc.ServiceError) => {
        // Clear the health-reset timer so a failing stream can't accidentally
        // reset the backoff counter before the next reconnect attempt.
        if (_streamHealthTimer !== null) {
          clearTimeout(_streamHealthTimer);
          _streamHealthTimer = null;
        }
        _singletonActive = false;
        _singletonCancelFn = null;
        scheduleReconnect(!!err);
      },
    );
  } catch (err) {
    _singletonActive = false;
    scheduleReconnect(true);
    logger.warn({ err }, "gRPC streamOrders threw synchronously — Rust engine likely not running");
  }
}

// ── WebSocket server ──────────────────────────────────────────────────────────
const wss = new WebSocketServer({ noServer: true });

const WS_PING_INTERVAL_MS = 20_000;

wss.on("connection", (ws: WebSocket, subscribedOrderIds: string[]) => {
  logger.info({ subscribedOrderIds }, "WS client connected to /api/bot/stream");

  let filterIds = new Set<string>(subscribedOrderIds);

  // Keepalive: ping every 20 s so the Replit proxy never silently drops the
  // connection due to inactivity. Terminate clients that miss a pong.
  let isAlive = true;
  ws.on("pong", () => { isAlive = true; });
  const pingTimer = setInterval(() => {
    if (!isAlive) { ws.terminate(); return; }
    isAlive = false;
    ws.ping();
  }, WS_PING_INTERVAL_MS);

  const onOrder = (payload: LiveTradePayload) => {
    if (ws.readyState !== ws.OPEN) return;
    if (filterIds.size > 0 && !filterIds.has(payload.id)) return;
    ws.send(JSON.stringify(payload));
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
    clearInterval(pingTimer);
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

// ── Python strategy engine (production only) ──────────────────────────────────
// In production, the Python engine runs as a child process alongside Node.js
// so the entire application is served from a single port (8080).
// In development each service runs independently via its own workflow.
if (process.env["NODE_ENV"] === "production") {
  // The bundle lives at <workspace>/artifacts/api-server/dist/index.mjs in
  // both prod and dev (pnpm dev runs build then start), so anchoring the
  // workspace root to the running file's location is stable regardless of
  // which directory pnpm/Node was launched from.
  const moduleDir = path.dirname(fileURLToPath(import.meta.url));
  const workspaceRoot = path.resolve(moduleDir, "..", "..", "..");
  const pythonCwd = path.resolve(workspaceRoot, "python-strategy");
  const venvPython = path.resolve(workspaceRoot, ".pythonlibs", "bin", "python3");

  // Resolution order: explicit override → bundled venv interpreter →
  // python3 on PATH. Whichever wins is logged so packaging regressions
  // surface immediately in the first deploy log line.
  let pythonBin: string;
  let pythonBinSource: string;
  if (process.env["PYTHON_BIN"]) {
    pythonBin = process.env["PYTHON_BIN"];
    pythonBinSource = "PYTHON_BIN env";
  } else if (existsSync(venvPython)) {
    pythonBin = venvPython;
    pythonBinSource = ".pythonlibs/bin/python3";
  } else {
    pythonBin = "python3";
    pythonBinSource = "PATH";
  }

  const cwdExists = existsSync(pythonCwd);
  if (!cwdExists) {
    logger.warn(
      { pythonCwd, workspaceRoot, moduleDir },
      "python-strategy directory not found — engine will fail to spawn",
    );
  }

  // Pre-spawn interpreter existence check. We can only stat absolute or
  // relative paths; bare "python3" must be left to PATH lookup at spawn time.
  const binIsResolvablePath = path.isAbsolute(pythonBin) || pythonBin.includes("/");
  const binExists = binIsResolvablePath ? existsSync(pythonBin) : null;
  if (binIsResolvablePath && binExists === false) {
    logger.warn(
      { pythonBin, binSource: pythonBinSource },
      "Configured Python interpreter path does not exist — engine will fail to spawn",
    );
  }
  logger.info(
    { pythonBin, binSource: pythonBinSource, binExists, pythonCwd, cwdExists, workspaceRoot },
    "Python strategy engine resolved paths",
  );

  setPythonStatus({
    state: "starting",
    bin: pythonBin,
    binSource: pythonBinSource,
    cwd: pythonCwd,
    cwdExists,
    binExists,
  });

  const py = spawn(pythonBin, ["main.py"], {
    cwd: pythonCwd,
    stdio: "inherit",
    env: { ...process.env, PORT: "8001" },
  });
  py.on("spawn", () => {
    logger.info(
      { cwd: pythonCwd, bin: pythonBin, binSource: pythonBinSource, pid: py.pid },
      "Python strategy engine started",
    );
    setPythonStatus({
      state: "running",
      pid: py.pid ?? null,
      startedAt: new Date().toISOString(),
    });
  });
  py.on("error", (err) => {
    logger.warn(
      { err: err.message, bin: pythonBin, binSource: pythonBinSource, cwd: pythonCwd, cwdExists },
      "Python strategy engine failed to start — API continues without it",
    );
    setPythonStatus({ state: "failed", lastError: err.message });
  });
  py.on("exit", (code, signal) => {
    logger.warn({ code, signal, bin: pythonBin }, "Python strategy engine exited");
    setPythonStatus({
      state: "exited",
      exitCode: code,
      exitSignal: signal,
      exitedAt: new Date().toISOString(),
      pid: null,
    });
  });
}

// Start the singleton gRPC stream once at server boot
startSingletonStream();

httpServer.listen(port, () => {
  logger.info({ port }, "Server listening (HTTP + WebSocket)");
});
