# Runbook: Key Rotation

**Scope**: Rotating the wallet private key, admin API key, and RPC credentials without downtime.

---

## When to Rotate

- Suspected compromise of any secret
- Scheduled quarterly rotation policy
- Team member offboarding
- CI/CD pipeline secret leak

---

## Keys in Scope

| Secret | Used By | Where Stored |
|--------|---------|--------------|
| `WALLET_PRIVATE_KEY` | Rust engine (signing) | Replit Secrets |
| `ADMIN_API_KEY` | Express API + dashboard | Replit Secrets |
| `SOLANA_RPC_URL` | Rust engine + Python | Replit Secrets |
| `JITO_AUTH_KEY` | Rust engine | Replit Secrets |
| `DATABASE_URL` | All services | Replit Secrets |

---

## Procedure: Admin API Key Rotation

This is low-risk since it does not affect on-chain assets.

1. **Generate new key** (32+ random characters):
   ```bash
   openssl rand -base64 32
   ```

2. **Update Replit Secret** `ADMIN_API_KEY` to the new value via Replit dashboard or CLI.

3. **Restart all services** (the key is read at startup):
   - `artifacts/api-server: API Server`
   - `python-strategy: Strategy Engine`

4. **Clear dashboard session cache**: Inform users to clear their browser session (the 1-hour TTL cache in `sessionStorage` will expire automatically).

5. **Verify** by hitting a protected endpoint:
   ```bash
   curl -X PUT http://localhost:8080/api/strategy/preset \
     -H "X-Admin-Key: NEW_KEY_HERE" \
     -H "Content-Type: application/json" \
     -d '{"preset":"balanced"}'
   ```

---

## Procedure: Wallet Private Key Rotation

**HIGH RISK** — this changes the signing wallet. Transfer any remaining funds before rotating.

1. **Pause all strategies** via the dashboard → Wallets → Pause.

2. **Drain the wallet**: transfer SOL and any token positions to a safe address using Phantom or Solflare.

3. **Generate a new keypair**:
   ```bash
   solana-keygen new --outfile new-keypair.json
   solana address -k new-keypair.json
   ```

4. **Fund the new wallet** with enough SOL for fees + trading capital.

5. **Update Replit Secret** `WALLET_PRIVATE_KEY` with the new base58 private key.

6. **Restart the Rust engine** workflow: `rust-engine: Trading Engine`.

7. **Verify** the new wallet address appears in the dashboard status bar.

8. **Securely delete** the old key file: `shred -u old-keypair.json`.

9. **Update the wallet record** in the database if `wallet_id` is tied to the old pubkey.

---

## Procedure: DATABASE_URL Rotation

1. **Create a new database user** in PostgreSQL:
   ```sql
   CREATE USER pumpy_new WITH PASSWORD 'new-strong-password';
   GRANT ALL PRIVILEGES ON DATABASE pumpy TO pumpy_new;
   ```

2. **Update `DATABASE_URL`** in Replit Secrets with the new credentials.

3. **Restart all services** that use the database (API server, Python engine).

4. **Verify** DB connectivity:
   ```bash
   curl http://localhost:8080/api/health
   # Expect: {"status":"ok","db":"connected"}
   ```

5. **Revoke the old user** after confirming all services are healthy:
   ```sql
   DROP USER pumpy_old;
   ```

---

## Post-Rotation Checklist

- [ ] New admin key accepted by protected endpoints
- [ ] New wallet address shown in dashboard
- [ ] No `UNAUTHORIZED` errors in API server logs
- [ ] Python engine reconnected to Rust gRPC (check circuit breaker state = CLOSED)
- [ ] Jito bundles landing (check MEV stats panel)
- [ ] Old keys invalidated / revoked
