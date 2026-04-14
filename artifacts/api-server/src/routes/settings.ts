import { Router } from "express";
import { readFileSync } from "fs";
import { db, botConfigTable } from "@workspace/db";
import { eq } from "drizzle-orm";

const router = Router();

// ── Constants ─────────────────────────────────────────────────────────────────

const ALLOWED_CONFIG_KEYS = new Set([
  "SOLANA_RPC_URL",
  "SOLANA_RPC_URLS",
  "JITO_BUNDLE_URL",
  "MAX_POSITION_SIZE_SOL",
  "STOP_LOSS_PERCENT",
  "TAKE_PROFIT_PERCENT",
  "RUST_GRPC_URL",
  "PYTHON_STRATEGY_URL",
]);

// ── Base58 helpers (no external deps) ────────────────────────────────────────

const BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

function base58Encode(bytes: Uint8Array): string {
  const digits: number[] = [0];
  for (const byte of bytes) {
    let carry = byte;
    for (let j = 0; j < digits.length; j++) {
      carry += digits[j]! << 8;
      digits[j] = carry % 58;
      carry = Math.floor(carry / 58);
    }
    while (carry > 0) {
      digits.push(carry % 58);
      carry = Math.floor(carry / 58);
    }
  }
  let result = "";
  for (let k = 0; bytes[k] === 0 && k < bytes.length - 1; k++) {
    result += BASE58_ALPHABET[0];
  }
  for (const d of digits.reverse()) {
    result += BASE58_ALPHABET[d]!;
  }
  return result;
}

function base58Decode(str: string): Uint8Array {
  const bytes = [0];
  for (const c of str) {
    const idx = BASE58_ALPHABET.indexOf(c);
    if (idx < 0) throw new Error(`Invalid base58 character: ${c}`);
    let carry = idx;
    for (let j = 0; j < bytes.length; j++) {
      carry += bytes[j]! * 58;
      bytes[j] = carry & 0xff;
      carry >>= 8;
    }
    while (carry > 0) {
      bytes.push(carry & 0xff);
      carry >>= 8;
    }
  }
  for (let k = 0; str[k] === "1" && k < str.length - 1; k++) {
    bytes.push(0);
  }
  return new Uint8Array(bytes.reverse());
}

// ── Pubkey extraction ─────────────────────────────────────────────────────────

function pubkeyFromKeypairBytes(bytes: Uint8Array): string | null {
  if (bytes.length !== 64) return null;
  return base58Encode(bytes.slice(32, 64));
}

function deriveWalletPubkey(): { pubkey: string | null; source: string | null } {
  const rawKey = process.env.WALLET_PRIVATE_KEY;
  if (rawKey) {
    try {
      const trimmed = rawKey.trim();
      let bytes: Uint8Array;
      if (trimmed.startsWith("[")) {
        bytes = new Uint8Array(JSON.parse(trimmed) as number[]);
      } else {
        bytes = base58Decode(trimmed);
      }
      return { pubkey: pubkeyFromKeypairBytes(bytes), source: "WALLET_PRIVATE_KEY" };
    } catch {
      return { pubkey: null, source: "WALLET_PRIVATE_KEY" };
    }
  }

  const keypairPath = process.env.KEYPAIR_PATH;
  if (keypairPath) {
    try {
      const raw = readFileSync(keypairPath, "utf8").trim();
      const bytes = new Uint8Array(JSON.parse(raw) as number[]);
      return { pubkey: pubkeyFromKeypairBytes(bytes), source: "KEYPAIR_PATH" };
    } catch {
      return { pubkey: null, source: "KEYPAIR_PATH" };
    }
  }

  return { pubkey: null, source: null };
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function maskString(val: string | undefined): string {
  if (!val) return "";
  if (val.length <= 8) return "****";
  return val.slice(0, 4) + "****" + val.slice(-4);
}

/**
 * Check that a URL is a safe external target before using it in a server-side fetch.
 * Rejects private/loopback IP ranges to prevent SSRF.
 */
function isSafeRpcUrl(urlStr: string): boolean {
  let parsed: URL;
  try {
    parsed = new URL(urlStr);
  } catch {
    return false;
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return false;
  const host = parsed.hostname.toLowerCase();
  // Block loopback and link-local hostnames
  if (host === "localhost" || host === "0.0.0.0" || host.endsWith(".local")) return false;
  // Block private IPv4 ranges and loopback
  const ipv4Parts = host.split(".").map(Number);
  if (ipv4Parts.length === 4 && ipv4Parts.every((n) => !isNaN(n))) {
    const [a, b] = ipv4Parts;
    if (a === 127) return false;                      // 127.0.0.0/8 loopback
    if (a === 10) return false;                       // 10.0.0.0/8 private
    if (a === 172 && b! >= 16 && b! <= 31) return false; // 172.16.0.0/12 private
    if (a === 192 && b === 168) return false;         // 192.168.0.0/16 private
    if (a === 169 && b === 254) return false;         // 169.254.0.0/16 link-local
    if (a === 0) return false;                        // 0.0.0.0/8 reserved
  }
  return true;
}

async function pingRpc(url: string): Promise<number | null> {
  try {
    const start = Date.now();
    const res = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "getHealth" }),
      signal: AbortSignal.timeout(5000),
    });
    if (!res.ok) return null;
    return Date.now() - start;
  } catch {
    return null;
  }
}

async function loadDbConfig(): Promise<Record<string, string>> {
  try {
    const rows = await db.select().from(botConfigTable);
    return Object.fromEntries(rows.map((r) => [r.key, r.value]));
  } catch {
    return {};
  }
}

// ── Env-var catalogue ─────────────────────────────────────────────────────────

interface EnvVarDef {
  key: string;
  required: boolean;
  description: string;
  setIn: string;
}

const ENV_VAR_DEFS: EnvVarDef[] = [
  {
    key: "SOLANA_RPC_URL",
    required: true,
    description: "Solana JSON-RPC endpoint (e.g. your Helius / QuickNode URL).",
    setIn: "Settings page or Replit Secrets panel",
  },
  {
    key: "SOLANA_RPC_URLS",
    required: false,
    description: "Comma-separated RPC endpoints for automatic failover.",
    setIn: "Settings page or Replit Secrets panel",
  },
  {
    key: "WALLET_PRIVATE_KEY",
    required: true,
    description: "Wallet private key as a base58 string or JSON byte array (64 bytes).",
    setIn: "Replit Secrets panel",
  },
  {
    key: "KEYPAIR_PATH",
    required: false,
    description: "Path to a Solana keypair JSON file on disk.",
    setIn: "Replit Files panel — upload the file, then set the path here",
  },
  {
    key: "DATABASE_URL",
    required: true,
    description: "PostgreSQL connection string. Auto-provided when you attach a Replit database.",
    setIn: "Replit Database tab (auto-injected)",
  },
  {
    key: "JITO_BUNDLE_URL",
    required: false,
    description: "Jito MEV bundle submission endpoint. Enables front-running protection.",
    setIn: "Settings page or Replit Secrets panel",
  },
  {
    key: "PYTHON_STRATEGY_URL",
    required: false,
    description: "URL of the Python strategy engine. Defaults to http://localhost:8001.",
    setIn: "Settings page or Replit Secrets panel",
  },
  {
    key: "RUST_GRPC_URL",
    required: false,
    description: "gRPC address of the Rust trading engine. Defaults to localhost:50051.",
    setIn: "Settings page or Replit Secrets panel",
  },
  {
    key: "GRPC_PORT",
    required: false,
    description: "Port the Rust gRPC server listens on (default: 50051).",
    setIn: "Replit Secrets panel",
  },
  {
    key: "METRICS_PORT",
    required: false,
    description: "Port for the Prometheus /metrics endpoint (default: 9091).",
    setIn: "Replit Secrets panel",
  },
  {
    key: "MAX_POSITION_SIZE_SOL",
    required: false,
    description: "Maximum SOL amount allowed per individual trade position.",
    setIn: "Settings page or Replit Secrets panel",
  },
  {
    key: "STOP_LOSS_PERCENT",
    required: false,
    description: "Stop-loss exit threshold as a percentage (e.g. 10 = exit at −10%).",
    setIn: "Settings page or Replit Secrets panel",
  },
  {
    key: "TAKE_PROFIT_PERCENT",
    required: false,
    description: "Take-profit exit threshold as a percentage (e.g. 50 = exit at +50%).",
    setIn: "Settings page or Replit Secrets panel",
  },
];

// ── Routes ─────────────────────────────────────────────────────────────────────

router.get("/settings/status", async (_req, res) => {
  const [walletInfo, dbConfig] = await Promise.all([
    Promise.resolve(deriveWalletPubkey()),
    loadDbConfig(),
  ]);

  const rpcUrl =
    dbConfig["SOLANA_RPC_URL"] ||
    process.env.SOLANA_RPC_URL ||
    dbConfig["SOLANA_RPC_URLS"]?.split(",")[0]?.trim() ||
    process.env.SOLANA_RPC_URLS?.split(",")[0]?.trim() ||
    null;

  const rpcLatencyMs = rpcUrl ? await pingRpc(rpcUrl) : null;

  const secretKeys = new Set(["WALLET_PRIVATE_KEY", "DATABASE_URL"]);

  const envVars = ENV_VAR_DEFS.map((def) => {
    const envVal = process.env[def.key];
    const dbVal = ALLOWED_CONFIG_KEYS.has(def.key) ? dbConfig[def.key] : undefined;
    const effectiveVal = dbVal || envVal;
    const isSecret = secretKeys.has(def.key);
    return {
      key: def.key,
      required: def.required,
      description: def.description,
      setIn: def.setIn,
      set: !!effectiveVal,
      source: dbVal ? "db" : envVal ? "env" : null,
      masked: effectiveVal
        ? isSecret
          ? "****"
          : maskString(effectiveVal)
        : "",
    };
  });

  res.json({
    wallet: {
      pubkey: walletInfo.pubkey,
      source: walletInfo.source,
      configured: !!(process.env.WALLET_PRIVATE_KEY || process.env.KEYPAIR_PATH),
    },
    rpc: {
      url: rpcUrl ? maskString(rpcUrl) : null,
      configured: !!rpcUrl,
      latencyMs: rpcLatencyMs,
      online: rpcLatencyMs !== null,
    },
    envVars,
  });
});

router.get("/settings/config", async (_req, res) => {
  try {
    const dbConfig = await loadDbConfig();
    const merged: Record<string, string> = {};
    for (const key of ALLOWED_CONFIG_KEYS) {
      const dbVal = dbConfig[key];
      const envVal = process.env[key];
      if (dbVal !== undefined) {
        merged[key] = dbVal;
      } else if (envVal !== undefined) {
        merged[key] = envVal;
      }
    }
    res.json(merged);
  } catch (err) {
    console.error("GET /settings/config error:", err);
    res.status(500).json({ error: "Failed to load config" });
  }
});

router.put("/settings/config", async (req, res) => {
  try {
    const body = req.body as Record<string, string>;
    if (!body || typeof body !== "object") {
      res.status(400).json({ error: "Request body must be a JSON object" });
      return;
    }
    const entries = Object.entries(body).filter(
      ([k, v]) => ALLOWED_CONFIG_KEYS.has(k) && typeof v === "string"
    );
    if (entries.length === 0) {
      res.status(400).json({ error: "No valid config keys provided" });
      return;
    }
    for (const [key, value] of entries) {
      if (value === "") {
        await db.delete(botConfigTable).where(eq(botConfigTable.key, key));
      } else {
        await db
          .insert(botConfigTable)
          .values({ key, value, updatedAt: new Date() })
          .onConflictDoUpdate({
            target: botConfigTable.key,
            set: { value, updatedAt: new Date() },
          });
      }
    }
    res.json({ ok: true, updated: entries.map(([k]) => k) });
  } catch (err) {
    console.error("PUT /settings/config error:", err);
    res.status(500).json({ error: "Failed to save config" });
  }
});

router.post("/settings/config/test-rpc", async (req, res) => {
  try {
    const { url } = req.body as { url?: string };
    if (!url || typeof url !== "string") {
      res.status(400).json({ ok: false, error: "A URL string is required" });
      return;
    }
    if (!isSafeRpcUrl(url)) {
      res.status(400).json({
        ok: false,
        error: "URL must be an external https:// or http:// address. Private/local addresses are not allowed.",
      });
      return;
    }
    const latencyMs = await pingRpc(url);
    res.json({ ok: latencyMs !== null, latencyMs });
  } catch (err) {
    console.error("POST /settings/config/test-rpc error:", err);
    res.status(500).json({ ok: false, error: "Test failed" });
  }
});

export default router;
