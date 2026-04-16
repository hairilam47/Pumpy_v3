#!/usr/bin/env bash

# Start Python strategy engine in the background.
# Errors here are non-fatal — the API server falls back to demo mode.
echo "==> Starting Python strategy engine..."
(cd python-strategy && python main.py) &
PYTHON_PID=$!

# Give Python a moment to start before Node.js begins accepting traffic.
sleep 2

echo "==> Starting Node.js API server (PORT=${PORT:-8080})..."
exec node --enable-source-maps artifacts/api-server/dist/index.mjs
