#!/usr/bin/env bash
set -e

echo "==> Installing Python dependencies..."
pip install -r python-strategy/requirements.txt --quiet --no-warn-script-location

echo "==> Building Node.js API server..."
pnpm --filter @workspace/api-server run build

echo "==> Build complete."
