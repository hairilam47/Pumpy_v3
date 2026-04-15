# Runbook: Backup & Restore

**Scope**: Database backup, configuration export, and disaster recovery procedures.

---

## What to Back Up

| Asset | Criticality | Frequency |
|-------|-------------|-----------|
| PostgreSQL database | Critical | Daily |
| `bot_config` table | Critical | On every change |
| `wallet_config` table | High | On every change |
| Replit Secrets | Critical | On every rotation |
| ML model file (`models/signal_model.joblib`) | Medium | After each training run |

---

## Backup: PostgreSQL Database

### Manual snapshot

```bash
# Full database dump (all tables)
pg_dump $DATABASE_URL -Fc -f backup_$(date +%Y%m%d_%H%M%S).dump

# Config tables only (smaller, faster)
pg_dump $DATABASE_URL -Fc -t bot_config -t wallet_config -t wallets \
  -f config_backup_$(date +%Y%m%d_%H%M%S).dump
```

### Automated daily backup script

```bash
#!/usr/bin/env bash
# Save as docs/scripts/backup.sh
set -euo pipefail

BACKUP_DIR="${BACKUP_DIR:-/tmp/pumpy_backups}"
RETENTION_DAYS="${RETENTION_DAYS:-7}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

mkdir -p "$BACKUP_DIR"

echo "[backup] Starting database backup at $TIMESTAMP"
pg_dump "$DATABASE_URL" -Fc -f "$BACKUP_DIR/pumpy_$TIMESTAMP.dump"
echo "[backup] Dump written to $BACKUP_DIR/pumpy_$TIMESTAMP.dump"

# Clean up old backups beyond retention window
find "$BACKUP_DIR" -name "pumpy_*.dump" -mtime "+$RETENTION_DAYS" -delete
echo "[backup] Removed backups older than $RETENTION_DAYS days"

# Backup ML model if present
MODEL_PATH="${MODEL_PATH:-python-strategy/models/signal_model.joblib}"
if [ -f "$MODEL_PATH" ]; then
  cp "$MODEL_PATH" "$BACKUP_DIR/signal_model_$TIMESTAMP.joblib"
  echo "[backup] ML model backed up"
fi

echo "[backup] Done"
```

Make executable: `chmod +x docs/scripts/backup.sh`

Schedule with cron (if available):
```bash
0 3 * * * /home/runner/workspace/docs/scripts/backup.sh >> /tmp/backup.log 2>&1
```

---

## Restore: PostgreSQL Database

```bash
# Restore full database (DESTRUCTIVE — stops all services first)
pg_restore -d $DATABASE_URL --clean --if-exists backup_YYYYMMDD_HHMMSS.dump

# Restore config tables only
pg_restore -d $DATABASE_URL --clean --if-exists -t bot_config -t wallet_config \
  config_backup_YYYYMMDD_HHMMSS.dump
```

After restoring, restart all services to pick up the new config.

---

## Restore: ML Model

```bash
cp signal_model_YYYYMMDD_HHMMSS.joblib python-strategy/models/signal_model.joblib
```

The Python engine will auto-reload the model on the next strategy cycle (using `reload_if_stale()`).

---

## Disaster Recovery Checklist

Use this checklist when recovering from a full environment loss:

1. **Provision new Replit environment** from the GitHub repo
2. **Restore all Replit Secrets** from your secure secrets manager
3. **Restore PostgreSQL** from the latest backup dump
4. **Run schema migration**: `pnpm --filter @workspace/db run push`
5. **Restore ML model** to `python-strategy/models/signal_model.joblib`
6. **Start all workflows** in order:
   - `rust-engine: Trading Engine`
   - `python-strategy: Strategy Engine`
   - `artifacts/api-server: API Server`
   - `artifacts/dashboard: web`
7. **Verify** all services green in the dashboard
8. **Verify** wallet balance matches expected value
9. **Run a test order** in demo mode if available

---

## Backup Verification

After each backup, verify integrity:

```bash
pg_restore --list backup_YYYYMMDD_HHMMSS.dump | head -20
# Should show table names and row counts
```

Test restore to a staging database monthly:
```bash
createdb pumpy_test
pg_restore -d pumpy_test backup_YYYYMMDD_HHMMSS.dump
psql pumpy_test -c "SELECT COUNT(*) FROM trades;"
dropdb pumpy_test
```
