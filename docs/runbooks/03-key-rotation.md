# Runbook: Compromised Key Rotation

Use this runbook when a wallet keypair or the admin API key may have been exposed. Act quickly — every minute of delay gives an attacker more time.

---

## Triage checklist

Before starting, confirm the scope of compromise:

- [ ] Is it a **wallet keypair** (private key / keypair file used for signing trades)?
- [ ] Is it the **admin API key** (`ADMIN_API_KEY` used to call protected endpoints)?
- [ ] Is it both?

Handle wallet keypair rotation (Part A) and admin key rotation (Part B) independently — they do not depend on each other.

---

## Part A — Wallet Keypair Rotation

### Step A1 — Halt the affected wallet immediately

This stops the bot from signing any further transactions with the compromised key while you rotate.

```bash
curl -s -X POST \
  -H "x-admin-key: $ADMIN_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{ "confirm": true }' \
  http://localhost:3001/api/wallets/{walletId}/halt
```

Expected response:

```json
{ "ok": true, "walletId": "wallet_abc123", "status": "halted" }
```

Verify the halt took effect:

```bash
curl -s -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/wallets \
  | jq '.[] | select(.walletId == "{walletId}") | .status'
# Should print: "halted"
```

---

### Step A2 — Drain funds from the compromised wallet

Move all SOL and token balances out of the compromised wallet to a safe address **before** the attacker can. Use the Solana CLI or your hardware wallet interface:

```bash
# Transfer all SOL (leave ~0.001 SOL for fees if needed)
solana transfer --from /path/to/compromised-keypair.json \
  <SAFE_DESTINATION_ADDRESS> ALL \
  --url mainnet-beta \
  --allow-unfunded-recipient
```

For SPL tokens, close each token account:

```bash
spl-token close --owner /path/to/compromised-keypair.json \
  <TOKEN_ACCOUNT_ADDRESS> \
  --recipient <SAFE_DESTINATION_ADDRESS>
```

---

### Step A3 — Generate a new keypair

```bash
# Generate a new keypair file
solana-keygen new --outfile /secure/path/new-wallet.json --no-passphrase

# Record the new public key
solana-keygen pubkey /secure/path/new-wallet.json
```

Store the new keypair file in a location accessible only by the bot process. Restrict permissions:

```bash
chmod 600 /secure/path/new-wallet.json
chown botuser:botuser /secure/path/new-wallet.json
```

Fund the new wallet with the SOL transferred from the old wallet.

---

### Step A4 — Update the keypair path in the database and environment

The `wallet_registry` table stores the `keypair_path` for each wallet. The settings API does not expose `keypair_path` for security reasons, so update it directly via `psql`:

```bash
psql "$DATABASE_URL" -c "
  UPDATE wallet_registry
  SET keypair_path = '/secure/path/new-wallet.json',
      owner_pubkey = '<NEW_WALLET_PUBLIC_KEY>'
  WHERE wallet_id = '{walletId}';
"
```

If the private key is stored as the `KEYPAIR_PATH` or `WALLET_PRIVATE_KEY` environment variable instead of a file, update the secret in your environment secrets panel (Replit: **Tools → Secrets**; other hosts: secret manager / `.env` file). These variables are not in the settings API's allowed key list and must be changed at the infrastructure level.

After updating the environment variable, **restart the API server and Rust engine** so they reload the value:

```bash
# In Replit: restart the "artifacts/api-server: API Server" workflow
# and the "rust-engine: Trading Engine" workflow
```

#### Verify the new keypair is loaded

Confirm the database update took effect and the new public key is stored:

```bash
psql "$DATABASE_URL" -c "
  SELECT wallet_id, owner_pubkey, keypair_path
  FROM wallet_registry
  WHERE wallet_id = '{walletId}';
"
```

Cross-check that `owner_pubkey` matches the public key printed by `solana-keygen pubkey /secure/path/new-wallet.json` in Step A3.

You can also check `GET /api/settings/status`, which shows the wallet pubkey derivation source and masked RPC config:

```bash
curl -s -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/settings/status | jq '.wallet'
# Returns: { "pubkey": "...", "source": "env" | "keypair", "configured": true }
```

---

### Step A5 — Resume the wallet after verification

Once the new keypair is confirmed active:

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

Expected response: `{ "ok": true, "walletId": "...", "status": "enabled" }`

---

## Part B — Admin Key Rotation

The admin API key (`ADMIN_API_KEY`) gates the wallet control operations: pause, resume, halt, and wallet config updates. If this key is compromised, an attacker can resume halted wallets or modify per-wallet risk limits.

Note: global bot configuration (`PUT /api/settings/config`) is not protected by the admin key — an attacker with network access to the API server can update Jito and RPC config without it. Prioritize revoking the key and restricting network access simultaneously.

### Step B1 — Generate a new admin key

Use a cryptographically random value of at least 32 characters:

```bash
openssl rand -hex 32
# Example output: a3f9c2e1b7d84f0a1c2e3d4b5f6a7e8c9d0b1e2f3a4c5d6e7f8a9b0c1d2e3f4
```

---

### Step B2 — Update the environment variable

Set the new key in your environment secrets panel (Replit: **Tools → Secrets**; other hosts: secret manager). The variable name is `ADMIN_API_KEY`.

**Do not** commit the key to source control or log it.

Restart the API server after the update:

```bash
# In Replit: restart the "artifacts/api-server: API Server" workflow
# Outside Replit:
kill $(lsof -ti :3001) && npm run start
```

---

### Step B3 — Verify the old key is revoked and the new key works

```bash
# Old key should now be rejected
curl -s -H "x-admin-key: OLD_KEY" \
  http://localhost:3001/api/wallets \
  | jq '.error'
# Expected: "Unauthorized" or "Forbidden"

# New key should work
curl -s -H "x-admin-key: $ADMIN_API_KEY" \
  http://localhost:3001/api/wallets \
  | jq '.[0].status'
# Expected: wallet status value (e.g. "enabled"), not an error
```

---

## Post-incident actions

1. **Audit the alert history** — check whether the attacker triggered unexpected state changes. Query the `wallet_alerts` table for recent events:
   ```bash
   psql "$DATABASE_URL" -c "
     SELECT wallet_id, error_type, count, last_at, auto_paused_at
     FROM wallet_alerts
     ORDER BY created_at DESC
     LIMIT 20;
   "
   ```
2. **Review trade history** — look for unauthorized orders placed with the compromised key:
   ```bash
   psql "$DATABASE_URL" -c "
     SELECT id, wallet_id, side, amount_sol, status, created_at
     FROM trades
     WHERE wallet_id = '{walletId}'
     ORDER BY created_at DESC
     LIMIT 20;
   "
   ```
3. **Rotate any other secrets** that may have been stored alongside the compromised key (database password, RPC API keys).
4. **Document the incident** — record the timeline, root cause, and steps taken for the post-mortem.
5. **Take a fresh database backup** per the [Backup & Restore guide](../backup.md) after confirming the system is clean.
