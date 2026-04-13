import { Router } from "express";
import { readFileSync } from "fs";

const router = Router();

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

/**
 * Extract the 32-byte public key from a 64-byte Solana keypair buffer
 * and return it as a base58 string. Returns null on any parse failure.
 */
function pubkeyFromKeypairBytes(bytes: Uint8Array): string | null {
  if (bytes.length !== 64) return null;
  return base58Encode(bytes.slice(32, 64));
}

/**
 * Derive the wallet public key from whichever source is configured.
 *
 * Priority:
 *   1. WALLET_PRIVATE_KEY — JSON array or base58 string
 *   2. KEYPAIR_PATH       — JSON file on disk
 */
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
    setIn: "Replit Secrets panel",
  },
  {
    key: "SOLANA_RPC_URLS",
    required: false,
    description: "Comma-separated RPC endpoints for automatic failover. Used only when SOLANA_RPC_URL is not set.",
    setIn: "Replit Secrets panel",
  },
  {
    key: "WALLET_PRIVATE_KEY",
    required: true,
    description: "Wallet private key as a base58 string or JSON byte array (64 bytes). Preferred over KEYPAIR_PATH.",
    setIn: "Replit Secrets panel",
  },
  {
    key: "KEYPAIR_PATH",
    required: false,
    description: "Path to a Solana keypair JSON file on disk. Used only when WALLET_PRIVATE_KEY is not set.",
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
    description: "Jito MEV bundle submission endpoint. Enables front-running protection via Jito.",
    setIn: "Replit Secrets panel",
  },
  {
    key: "PYTHON_STRATEGY_URL",
    required: false,
    description: "URL of the Python strategy engine. Defaults to http://localhost:8001.",
    setIn: "Replit Secrets panel",
  },
  {
    key: "RUST_GRPC_URL",
    required: false,
    description: "gRPC address of the Rust trading engine. Defaults to localhost:50051.",
    setIn: "Replit Secrets panel",
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
    setIn: "Replit Secrets panel",
  },
  {
    key: "STOP_LOSS_PERCENT",
    required: false,
    description: "Stop-loss exit threshold as a percentage (e.g. 10 = exit at −10%).",
    setIn: "Replit Secrets panel",
  },
  {
    key: "TAKE_PROFIT_PERCENT",
    required: false,
    description: "Take-profit exit threshold as a percentage (e.g. 50 = exit at +50%).",
    setIn: "Replit Secrets panel",
  },
];

// ── Route ─────────────────────────────────────────────────────────────────────

router.get("/settings/status", async (_req, res) => {
  const walletInfo = deriveWalletPubkey();

  // SOLANA_RPC_URL is the canonical simple path; SOLANA_RPC_URLS is the advanced failover list.
  const rpcUrl =
    process.env.SOLANA_RPC_URL ||
    process.env.SOLANA_RPC_URLS?.split(",")[0]?.trim() ||
    null;

  const rpcLatencyMs = rpcUrl ? await pingRpc(rpcUrl) : null;

  const envVars = ENV_VAR_DEFS.map((def) => {
    const rawVal = process.env[def.key];
    const isSecret =
      def.key === "WALLET_PRIVATE_KEY" ||
      def.key === "DATABASE_URL";
    return {
      key: def.key,
      required: def.required,
      description: def.description,
      setIn: def.setIn,
      set: !!rawVal,
      masked: rawVal
        ? isSecret
          ? "****"
          : maskString(rawVal)
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

export default router;
