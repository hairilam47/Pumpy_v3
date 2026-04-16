# Runbook: Halted / Paused Wallet Recovery

Use this runbook when a wallet has been automatically or manually paused or halted and needs to be brought back into service safely.

---

## Terminology

| Status | Meaning |
|--------|---------|
| `enabled` | Wallet is active and trading. |
| `paused` | Wallet stopped automatically (daily loss limit hit, circuit breaker, anomaly). Trading is suspended but config is intact. |
| `halted` | Wallet stopped manually via the halt endpoint. Requires explicit confirmation to resume. |

---

## Step 1 — Identify affected wallets

### Via the API

```bash
curl -s -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/wallets \
  | jq '.[] | select(.status != "enabled") | {walletId, ownerPubkey, status}'
```

Expected response fields:

```json
{
  "walletId": "wallet_abc123",
  "ownerPubkey": "So11111...PublicKey",
  "status": "paused"
}
```

### Via the dashboard

Open the **Wallets** page. Any wallet with a yellow (paused) or red (halted) badge has stopped trading. The alert history panel shows the trigger reason and timestamp.

### Log patterns to look for

Search the API server logs for entries containing the wallet ID and terms like `pause`, `halt`, or `circuit_breaker`. In the Replit environment, open the `artifacts/api-server: API Server` workflow log and filter by the wallet's `walletId` or `ownerPubkey`.

---

## Step 2 — Investigate the root cause

Before resuming, understand _why_ the wallet stopped.

### Get the wallet's current config

```bash
# Wallet config (risk limits, preset, status)
curl -s -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/wallets/{walletId}/config
```

### Query the alert history directly from the database

The `wallet_alerts` table stores the error type, occurrence count, and auto-pause timestamp for each wallet:

```bash
psql "$DATABASE_URL" -c "
  SELECT error_type, count, last_at, auto_paused_at, created_at
  FROM wallet_alerts
  WHERE wallet_id = '{walletId}'
  ORDER BY created_at DESC
  LIMIT 10;
"
```

Columns:
- `error_type` — category of the error that triggered the alert (e.g. `daily_loss_limit_exceeded`, `circuit_breaker`)
- `count` — how many times this error type has occurred for this wallet
- `last_at` — timestamp of the most recent occurrence
- `auto_paused_at` — when the wallet was automatically paused (null if the wallet was not auto-paused)

### Common pause causes and checks

| Cause | What to check |
|-------|--------------|
| `daily_loss_limit_exceeded` | Review `dailyLossSol` vs `dailyLossLimitSol` in the config response. Consider raising the limit or reducing position size before resuming. |
| `circuit_breaker_triggered` | Look for repeated failed orders or high slippage in the `trades` table. Check if the token pair is still liquid. |
| `anomaly_detected` | Review the last few trades. Confirm the strategy is behaving as expected. |
| Manual halt | Confirm with the operator who halted it that the issue has been resolved. |

---

## Step 3 — Adjust risk config if needed

If the pause was due to a misconfigured risk limit, update it before resuming:

```bash
curl -s -X PUT \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "daily_loss_limit_sol": 0.5,
    "risk_per_trade_sol": 0.05
  }' \
  http://localhost:3001/api/wallets/{walletId}/config
```

Accepted config keys for `PUT /api/wallets/{walletId}/config`:

| Key | Description |
|-----|-------------|
| `daily_loss_limit_sol` | Max SOL loss per day before auto-pause triggers. |
| `risk_per_trade_sol` | Max SOL risked on a single trade. |
| `strategy_preset` | `conservative` / `balanced` / `aggressive` |
| `status` | Override status directly (`enabled` / `paused` / `halted`). |

---

## Step 4 — Resume the wallet

### Resume a paused wallet

```bash
curl -s -X POST \
  -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/wallets/{walletId}/resume
```

Expected response:

```json
{ "ok": true, "walletId": "wallet_abc123", "status": "enabled" }
```

### Resume a halted wallet

Halted wallets require you to set status back to `enabled` via the config endpoint first, then call resume:

```bash
curl -s -X PUT \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{ "status": "enabled" }' \
  http://localhost:3001/api/wallets/{walletId}/config

curl -s -X POST \
  -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/wallets/{walletId}/resume
```

---

## Step 5 — Verify the wallet is trading again

```bash
# Confirm status is back to enabled
curl -s -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/wallets \
  | jq '.[] | select(.walletId == "{walletId}") | .status'

# Check bot status for active strategies
curl -s http://localhost:3001/api/bot/status | jq '.activeStrategies'
```

Monitor the dashboard **Trade Feed** for new activity from the wallet over the next few minutes.

---

## Escalation

If the wallet re-pauses immediately after resuming, the underlying strategy or market condition has not changed. Do not resume repeatedly — investigate the strategy config and market conditions first. If a suspected exploit or anomaly is involved, halt the wallet and rotate the keypair per the [Key Rotation Runbook](./03-key-rotation.md).
