# Database Backup & Restore

This document covers how to back up the PostgreSQL database used by the PumpFun Trading Bot, how to restore it, and how to verify the restore was successful.

---

## What is backed up

The database holds all persistent operational state:

- `wallet_registry` — wallet IDs, keypair paths, owner public keys, and status (`enabled` / `paused` / `halted`)
- `wallet_config` — per-wallet risk limits (`risk_per_trade_sol`, `daily_loss_limit_sol`, `strategy_preset`)
- `strategies` — per-strategy trade counters, P&L accumulators, and config
- `bot_config` — global key/value config overrides (Jito tip bounds, RPC URLs, etc.)
- `trades` — executed order records, signatures, and P&L per trade
- `wallet_alerts` — wallet pause/halt events and circuit-breaker triggers

---

## Prerequisites

- `pg_dump` and `psql` must be installed on the host running the backup.  
  On Debian/Ubuntu: `apt-get install postgresql-client`
- The `DATABASE_URL` environment variable must be set, or supply the connection details explicitly.

```
# Example DATABASE_URL format
postgres://USER:PASSWORD@HOST:5432/DBNAME
```

---

## Creating a backup

### Automated backup script

A ready-made backup script is available at `docs/scripts/backup.sh`. It handles the database dump, an optional config-tables-only snapshot, ML model backup, and retention cleanup in one step:

```bash
# Set required env vars, then run:
DATABASE_URL="postgres://..." \
BACKUP_DIR="/backups/pumpfun" \
RETENTION_DAYS=30 \
bash docs/scripts/backup.sh
```

The script dumps the full database, verifies dump integrity via `pg_restore --list`, and removes files older than `RETENTION_DAYS`. Use this for both manual runs and scheduled cron jobs.

### One-off dump (manual)

```bash
pg_dump "$DATABASE_URL" \
  --format=custom \
  --no-owner \
  --no-acl \
  --file="pumpfun_$(date +%Y%m%d_%H%M%S).dump"
```

- `--format=custom` produces a compressed binary archive that `pg_restore` can load selectively.
- `--no-owner` and `--no-acl` make the dump portable across different DB users.

Expected output: no output on success. The file will be created in the current directory. Check its size — a healthy dump with a few days of trade history is typically 1–50 MB.

### Plain SQL dump (human-readable)

```bash
pg_dump "$DATABASE_URL" \
  --format=plain \
  --no-owner \
  --no-acl \
  --file="pumpfun_$(date +%Y%m%d_%H%M%S).sql"
```

Useful for inspecting or grepping the schema. Not recommended for large databases.

---

## Scheduling backups

Add a cron job on the host or ops VM. Edit the crontab with `crontab -e`:

```
# Daily backup at 03:00 UTC, keep 30 days
0 3 * * * pg_dump "$DATABASE_URL" --format=custom --no-owner --no-acl \
    --file="/backups/pumpfun_$(date +\%Y\%m\%d).dump" \
  && find /backups -name "pumpfun_*.dump" -mtime +30 -delete
```

Alternatively, if using a managed database (e.g., Neon, Supabase, Railway), enable the provider's point-in-time recovery or scheduled export feature from the dashboard instead.

---

## Restoring a backup

> **Warning:** Restoring overwrites the target database. Stop the API server and Python strategy engine before restoring to prevent write conflicts.

### 1. Stop services

```bash
# In the Replit environment — stop all workflows before restoring
# Outside Replit — stop the relevant processes
kill $(lsof -ti :3001)   # API server default port
```

### 2. Restore into the existing database (recommended)

Use `--clean --if-exists` to drop and recreate all objects inside the existing database without needing to drop the database itself. This avoids the "cannot drop the currently open database" error that occurs when `DATABASE_URL` points to the target database:

```bash
pg_restore \
  --dbname="$DATABASE_URL" \
  --clean \
  --if-exists \
  --no-owner \
  --no-acl \
  --exit-on-error \
  pumpfun_20240415_030000.dump
```

`--clean` drops each object before recreating it. `--if-exists` suppresses errors for objects that don't yet exist (useful when restoring into a brand-new database).

Expected output: none on success. Errors are printed to stderr — any error means the restore is incomplete; check the message and re-run.

### 2b. Restore into a fresh database (alternative)

If you need to restore into a completely empty database on a different cluster or host, connect to the PostgreSQL maintenance database (`postgres`) instead of the target to run the drop/create commands. Replace `<HOST>`, `<PORT>`, and `<USER>` with your cluster details:

```bash
# Connect to the maintenance DB to drop/create the target
psql "postgres://<USER>:<PASSWORD>@<HOST>:<PORT>/postgres" \
  -c "DROP DATABASE IF EXISTS <TARGET_DBNAME>;"
psql "postgres://<USER>:<PASSWORD>@<HOST>:<PORT>/postgres" \
  -c "CREATE DATABASE <TARGET_DBNAME>;"

# Then restore
pg_restore \
  --dbname="postgres://<USER>:<PASSWORD>@<HOST>:<PORT>/<TARGET_DBNAME>" \
  --no-owner \
  --no-acl \
  --exit-on-error \
  pumpfun_20240415_030000.dump
```

### 4. Verify the restore

Run spot checks immediately after restoring:

```bash
# Count rows in critical tables
psql "$DATABASE_URL" -c "SELECT COUNT(*) FROM wallet_registry;"
psql "$DATABASE_URL" -c "SELECT COUNT(*) FROM trades;"
psql "$DATABASE_URL" -c "SELECT MAX(executed_at) FROM trades;"

# Confirm latest wallet statuses look correct
psql "$DATABASE_URL" -c "SELECT wallet_id, owner_pubkey, status FROM wallet_registry;"
```

Cross-reference the row counts and the most-recent `executed_at` timestamp against what you expect from the dump date. If the counts are zero or the timestamp is wrong, the dump file may be corrupt — restore from the next most-recent backup.

### 5. Restart services

Once verification passes, restart the API server and strategy engine. The bot will read wallet status and config from the restored database on startup.

---

## Backup file storage recommendations

- Store backups in a location separate from the application host (S3, GCS, Backblaze B2, etc.).
- Encrypt the dump file before uploading — it contains wallet keypair paths and may contain sensitive config:
  ```bash
  gpg --symmetric --cipher-algo AES256 pumpfun_20240415.dump
  ```
- Retain at least 7 daily backups and 4 weekly backups.
