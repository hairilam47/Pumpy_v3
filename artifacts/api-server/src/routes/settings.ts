import { Router } from "express";

const router = Router();

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

function deriveWalletPubkey(): { pubkey: string | null; source: string | null } {
  const rawKey = process.env.WALLET_PRIVATE_KEY;
  if (rawKey) {
    try {
      const trimmed = rawKey.trim();
      let bytes: Uint8Array;
      if (trimmed.startsWith("[")) {
        const arr: number[] = JSON.parse(trimmed);
        bytes = new Uint8Array(arr);
      } else {
        bytes = base58Decode(trimmed);
      }
      if (bytes.length === 64) {
        const pubkeyBytes = bytes.slice(32, 64);
        return { pubkey: base58Encode(pubkeyBytes), source: "WALLET_PRIVATE_KEY" };
      }
    } catch {
    }
    return { pubkey: null, source: "WALLET_PRIVATE_KEY" };
  }
  if (process.env.KEYPAIR_PATH) {
    return { pubkey: null, source: "KEYPAIR_PATH" };
  }
  return { pubkey: null, source: null };
}

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
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "getHealth",
      }),
      signal: AbortSignal.timeout(5000),
    });
    if (!res.ok) return null;
    return Date.now() - start;
  } catch {
    return null;
  }
}

router.get("/settings/status", async (_req, res) => {
  const walletInfo = deriveWalletPubkey();

  const rpcUrl =
    process.env.SOLANA_RPC_URLS?.split(",")[0]?.trim() ||
    process.env.SOLANA_RPC_URL ||
    null;

  const rpcLatencyMs = rpcUrl ? await pingRpc(rpcUrl) : null;

  const envVars = [
    { key: "SOLANA_RPC_URL", set: !!process.env.SOLANA_RPC_URL, masked: maskString(process.env.SOLANA_RPC_URL) },
    { key: "SOLANA_RPC_URLS", set: !!process.env.SOLANA_RPC_URLS, masked: maskString(process.env.SOLANA_RPC_URLS) },
    { key: "WALLET_PRIVATE_KEY", set: !!process.env.WALLET_PRIVATE_KEY, masked: process.env.WALLET_PRIVATE_KEY ? "****" : "" },
    { key: "KEYPAIR_PATH", set: !!process.env.KEYPAIR_PATH, masked: maskString(process.env.KEYPAIR_PATH) },
    { key: "DATABASE_URL", set: !!process.env.DATABASE_URL, masked: process.env.DATABASE_URL ? "****" : "" },
    { key: "JITO_BUNDLE_URL", set: !!process.env.JITO_BUNDLE_URL, masked: maskString(process.env.JITO_BUNDLE_URL) },
    { key: "PYTHON_STRATEGY_URL", set: !!process.env.PYTHON_STRATEGY_URL, masked: maskString(process.env.PYTHON_STRATEGY_URL) },
    { key: "RUST_GRPC_URL", set: !!process.env.RUST_GRPC_URL, masked: maskString(process.env.RUST_GRPC_URL) },
    { key: "GRPC_PORT", set: !!process.env.GRPC_PORT, masked: process.env.GRPC_PORT || "" },
    { key: "METRICS_PORT", set: !!process.env.METRICS_PORT, masked: process.env.METRICS_PORT || "" },
    { key: "MAX_POSITION_SIZE_SOL", set: !!process.env.MAX_POSITION_SIZE_SOL, masked: process.env.MAX_POSITION_SIZE_SOL || "" },
    { key: "STOP_LOSS_PERCENT", set: !!process.env.STOP_LOSS_PERCENT, masked: process.env.STOP_LOSS_PERCENT || "" },
    { key: "TAKE_PROFIT_PERCENT", set: !!process.env.TAKE_PROFIT_PERCENT, masked: process.env.TAKE_PROFIT_PERCENT || "" },
  ];

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
