# Runbook: Jito Bundle Failures

**Scope**: MEV bundle submission failures resulting in orders falling back to standard RPC, or bundle rejections causing trades to not land.

---

## Symptoms

- Dashboard MEV stats: **Jito Bundles** count is 0 or stagnant while trades are executing
- Rust engine logs: `Jito bundle failed` or `jito_tip` errors
- Orders completed with `signature` field present but `mev_protected: false`
- High execution latency (>2 s) or multiple retries per order

---

## Diagnostic Steps

### 1. Check Jito tip configuration

```bash
curl http://localhost:8080/api/settings/status
# Check JITO_TIP_PERCENT, JITO_TIP_FLOOR, JITO_TIP_CEILING in envVars
```

Optimal tip range: 0.1% – 3.0% of transaction value. If floor > ceiling, bundles will always fail.

### 2. Review Rust engine logs

```bash
cat /tmp/logs/rust-engine_*.log | grep -i "jito\|bundle\|mev"
```

Common error patterns:
- `bundle rejected: insufficient tip` → Increase `JITO_TIP_FLOOR`
- `bundle timeout` → Jito block engine overloaded; check https://jito.network/status
- `no leader found` → RPC endpoint issue; verify `SOLANA_RPC_URL`

### 3. Test RPC endpoint connectivity

```bash
curl -X POST $SOLANA_RPC_URL \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[]}'
```

### 4. Check on-chain bundle landing rate

Visit: https://explorer.jito.wtf and search for recent tip payment accounts.

---

## Resolution

### Option A: Adjust tip parameters

```bash
# Via bot_config table
psql $DATABASE_URL -c "UPDATE bot_config SET value='0.5' WHERE key='JITO_TIP_PERCENT';"
psql $DATABASE_URL -c "UPDATE bot_config SET value='5000' WHERE key='JITO_TIP_FLOOR';"
psql $DATABASE_URL -c "UPDATE bot_config SET value='500000' WHERE key='JITO_TIP_CEILING';"
```

Then restart the Rust engine workflow to pick up new config values.

### Option B: Temporarily disable Jito (RPC fallback)

If Jito is experiencing widespread outages, set `USE_JITO=false` in Replit Secrets and restart the Rust engine. Orders will execute via standard RPC without MEV protection.

### Option C: Switch to backup RPC

1. Update `SOLANA_RPC_URL` in Replit Secrets to a backup provider (e.g., Helius, QuickNode)
2. Restart the Rust engine workflow

---

## Prevention

- Monitor the Jito status page during high-volatility periods
- Set `JITO_TIP_PERCENT` to auto-scale (default: 0.5%) and keep floor at 5000 lamports minimum
- Use the retry logic: the engine retries once after 2 s before falling back to RPC
- Check `sandwichAttacks` counter in the dashboard — if zero, MEV protection is working

---

## Escalation

1. If RPC fallback is also failing: check `SOLANA_RPC_URL` and Solana network status at https://status.solana.com
2. File a bug if Jito tip calculation produces values outside `[floor, ceiling]` range
