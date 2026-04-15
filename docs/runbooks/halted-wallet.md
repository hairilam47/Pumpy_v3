# Runbook: Halted Wallet

**Scope**: Auto-pause triggered by the bot's safety layer, or manual halt requested by an operator.

---

## Symptoms

- Dashboard shows wallet status as **PAUSED** or **HALTED**
- No new orders being submitted for the affected wallet
- Log line: `wallet auto-paused` or `resume required`

---

## Diagnostic Steps

### 1. Confirm the wallet is paused

```bash
curl http://localhost:8080/api/wallets/{walletId}/config
# Look for: "status": "paused"
```

### 2. Check recent trade history for the trigger

```bash
curl "http://localhost:8080/api/trades?wallet_id={walletId}&limit=20"
```

Look for:
- Consecutive losses (`pnl_sol < 0`) exceeding the stop-loss threshold
- A single large loss (`pnl_sol` << expected)
- `status: "Failed"` orders indicating execution errors

### 3. Check the bot status endpoint

```bash
curl http://localhost:8080/api/bot/status
# Review: grpcConnected, pythonEngineRunning, running
```

### 4. Review Python strategy engine logs

```bash
journalctl -u python-strategy --since "10 minutes ago"
# OR in development:
cat /tmp/logs/python-strategy_*.log | grep -i "pause\|halt\|auto"
```

---

## Resolution

### Option A: Resume via dashboard

1. Navigate to **Wallets** page
2. Find the halted wallet entry
3. Click **Resume** button
4. Enter admin key when prompted
5. Verify the status returns to **ACTIVE**

### Option B: Resume via API

```bash
curl -X POST http://localhost:8080/api/wallets/{walletId}/resume \
  -H "Content-Type: application/json" \
  -H "X-Admin-Key: $ADMIN_API_KEY" \
  -d '{}'
```

Expected response:
```json
{"success": true, "status": "active"}
```

---

## Prevention / Follow-up

- Review the loss threshold in `bot_config`: `stop_loss_pct`, `daily_loss_limit_sol`
- If losses were caused by a bad strategy signal, consider switching to a more conservative preset
- Check on-chain for sandwich attacks during the loss period
- Update the `strategy_preset` to `conservative` temporarily if market conditions are volatile

---

## Escalation

If the wallet cannot be resumed after following the above steps:
1. Verify `ADMIN_API_KEY` environment variable is set correctly
2. Check the Express API server health: `curl http://localhost:8080/api/health`
3. Restart the API server workflow: `artifacts/api-server: API Server`
