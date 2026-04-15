#!/usr/bin/env bash
# PumpyPumpyFunBotTrade — automated backup script
# Usage: ./docs/scripts/backup.sh
# Environment: DATABASE_URL, MODEL_PATH (optional), BACKUP_DIR (optional)

set -euo pipefail

BACKUP_DIR="${BACKUP_DIR:-/tmp/pumpy_backups}"
RETENTION_DAYS="${RETENTION_DAYS:-7}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
MODEL_PATH="${MODEL_PATH:-python-strategy/models/signal_model.joblib}"

mkdir -p "$BACKUP_DIR"

echo "[backup] ============================================================"
echo "[backup] PumpyPumpyFunBotTrade backup — $TIMESTAMP"
echo "[backup] ============================================================"

# ── PostgreSQL backup ─────────────────────────────────────────────────────────
if [ -z "${DATABASE_URL:-}" ]; then
  echo "[backup] ERROR: DATABASE_URL is not set — skipping database backup"
else
  DUMP_FILE="$BACKUP_DIR/pumpy_$TIMESTAMP.dump"
  echo "[backup] Dumping database to $DUMP_FILE ..."
  pg_dump "$DATABASE_URL" -Fc -f "$DUMP_FILE"
  DUMP_SIZE=$(du -sh "$DUMP_FILE" | cut -f1)
  echo "[backup] Database dump complete ($DUMP_SIZE)"

  # Verify integrity
  pg_restore --list "$DUMP_FILE" > /dev/null
  echo "[backup] Dump integrity verified"
fi

# ── Config tables only (smaller supplemental snapshot) ───────────────────────
if [ -n "${DATABASE_URL:-}" ]; then
  CONFIG_FILE="$BACKUP_DIR/config_$TIMESTAMP.dump"
  pg_dump "$DATABASE_URL" -Fc \
    -t bot_config -t wallet_config -t wallets \
    -f "$CONFIG_FILE" 2>/dev/null || \
    echo "[backup] WARN: Some config tables missing (skipped)"
  echo "[backup] Config snapshot: $CONFIG_FILE"
fi

# ── ML model backup ───────────────────────────────────────────────────────────
if [ -f "$MODEL_PATH" ]; then
  MODEL_DEST="$BACKUP_DIR/signal_model_$TIMESTAMP.joblib"
  cp "$MODEL_PATH" "$MODEL_DEST"
  echo "[backup] ML model backed up to $MODEL_DEST"
else
  echo "[backup] ML model not found at $MODEL_PATH — skipping"
fi

# ── Retention cleanup ─────────────────────────────────────────────────────────
echo "[backup] Removing backups older than $RETENTION_DAYS days..."
find "$BACKUP_DIR" -name "pumpy_*.dump" -mtime "+$RETENTION_DAYS" -delete
find "$BACKUP_DIR" -name "config_*.dump" -mtime "+$RETENTION_DAYS" -delete
find "$BACKUP_DIR" -name "signal_model_*.joblib" -mtime "+$RETENTION_DAYS" -delete

REMAINING=$(ls "$BACKUP_DIR" | wc -l)
echo "[backup] Backup directory now contains $REMAINING file(s)"

echo "[backup] ============================================================"
echo "[backup] Backup complete — $TIMESTAMP"
echo "[backup] ============================================================"
