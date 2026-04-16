# Runbook: Jito Bundle Failures & RPC Fallback

Use this runbook when Jito bundle submissions are being consistently rejected, orders are failing to land on-chain, or the RPC endpoint is unreachable.

---

## Symptoms

- Dashboard MEV stats show `bundlesLanded / bundlesSubmitted` ratio below 20%.
- Trade history shows orders stuck in `pending` or `failed` status.
- API server logs contain repeated error entries referencing bundle submission, simulation failures, or RPC connectivity (search for the wallet or order ID in the API server workflow log).
- `GET /api/bot/metrics` returns elevated `ordersFailed` and `rpcErrorRate`.

---

## Step 1 — Diagnose: check MEV stats and metrics

```bash
# Bundle landing rate and Jito status
curl -s http://localhost:3001/api/bot/mev-stats | jq .

# Overall order failure rate
curl -s http://localhost:3001/api/bot/metrics \
  | jq '{ordersFailed, ordersExecuted, rpcErrorRate}'
```

A healthy system shows `landedRate` above 50% and `rpcErrorRate` below 5%.

---

## Step 2 — Check current Jito and RPC configuration

```bash
curl -s -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/settings/config | jq .
```

Key fields to review:

| Config Key / Env Var | Description |
|---------------------|-------------|
| `JITO_BUNDLE_URL` | The Jito block engine endpoint (e.g., `https://mainnet.block-engine.jito.wtf`). |
| `JITO_TIP_FLOOR_LAMPORTS` | Minimum tip in lamports. If floor > ceiling, bundles are never sent. |
| `JITO_TIP_CEILING_LAMPORTS` | Maximum tip in lamports. |
| `JITO_TIP_PERCENT` | Fraction of trade value used as tip (e.g., `0.001` = 0.1%). |
| `JITO_SIMULATION_ENABLED` | If `true`, bundles are simulated before submission (slower but safer). |
| `SOLANA_RPC_URL` | Primary JSON-RPC endpoint. |
| `SOLANA_RPC_URLS` | Comma-separated fallback list. |

### Check tip floor vs ceiling

If `JITO_TIP_FLOOR_LAMPORTS` is greater than `JITO_TIP_CEILING_LAMPORTS`, the bot cannot construct a valid tip and silently skips bundle submission. The dashboard shows a warning banner for this condition.

Fix by updating the ceiling to be at least as large as the floor:

```bash
curl -s -X PUT \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "JITO_TIP_FLOOR_LAMPORTS": "1000",
    "JITO_TIP_CEILING_LAMPORTS": "50000"
  }' \
  http://localhost:3001/api/settings/config
```

---

## Step 3 — Test RPC connectivity

```bash
# Test the primary RPC endpoint
curl -s -X POST \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{ "url": "https://api.mainnet-beta.solana.com" }' \
  http://localhost:3001/api/settings/config/test-rpc | jq .
```

Expected response:

```json
{ "latencyMs": 120, "ok": true }
```

If `ok` is `false` or latency exceeds 1000 ms, the RPC endpoint is degraded. Proceed to Step 4.

---

## Step 4 — Switch to a fallback RPC

Update `SOLANA_RPC_URL` to a working endpoint. Use one of the providers in `SOLANA_RPC_URLS` if set, or substitute a new one:

```bash
curl -s -X PUT \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "SOLANA_RPC_URL": "https://your-fallback-rpc.example.com"
  }' \
  http://localhost:3001/api/settings/config
```

Test the new URL with the test-rpc endpoint (Step 3) before committing.

Common fallback RPC providers:

- Helius: `https://mainnet.helius-rpc.com/?api-key=YOUR_KEY`
- QuickNode: `https://YOUR_ENDPOINT.quiknode.pro/YOUR_TOKEN/`
- Triton: `https://YOUR_NODE.rpcpool.com/YOUR_TOKEN`
- Public (limited): `https://api.mainnet-beta.solana.com`

---

## Step 5 — Adjust Jito tip to improve landing rate

If the RPC is healthy but bundles still fail to land, the tip may be too low for current network congestion. Raise the tip ceiling and/or floor:

```bash
curl -s -X PUT \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "JITO_TIP_FLOOR_LAMPORTS": "5000",
    "JITO_TIP_CEILING_LAMPORTS": "100000",
    "JITO_TIP_PERCENT": "0.002"
  }' \
  http://localhost:3001/api/settings/config
```

Monitor `landedRate` via `GET /api/bot/mev-stats` after each adjustment. Allow 5–10 minutes of traffic for the rate to stabilize.

---

## Step 6 — Disable Jito (emergency direct-RPC mode)

If Jito's block engine itself is down (check https://status.jito.network), disable bundle submission to fall back to standard RPC transactions:

```bash
curl -s -X PUT \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{ "JITO_BUNDLE_URL": "" }' \
  http://localhost:3001/api/settings/config
```

Setting `JITO_BUNDLE_URL` to an empty string causes the Rust engine to submit orders as standard Solana transactions without MEV protection. Orders will land but are exposed to front-running. Re-enable Jito once the outage is resolved.

---

## Step 7 — Verify recovery

```bash
# Re-check MEV stats after 5 minutes
curl -s http://localhost:3001/api/bot/mev-stats | jq .

# Confirm orders are landing
curl -s http://localhost:3001/api/bot/metrics \
  | jq '{ordersSubmitted, ordersExecuted, ordersFailed}'
```

If `bundlesLanded` begins rising and `ordersFailed` is decreasing, the system has recovered. If failures continue, escalate to the node provider's support channel with timestamps and RPC error messages from the API server logs.
