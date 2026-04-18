#!/usr/bin/env bash
set -e

echo "==> Installing Node.js dependencies..."
pnpm install --frozen-lockfile

echo "==> Installing Python dependencies..."
pip install -r python-strategy/requirements.txt --quiet --no-warn-script-location

echo "==> Building dashboard (static files served by Express in production)..."
BASE_PATH="/dashboard/" PORT=23183 \
  pnpm --filter @workspace/dashboard run build

echo "==> Building Node.js API server..."
pnpm --filter @workspace/api-server run build

echo "==> Verifying production deploy artifacts..."
# Guard against silent regressions in .replitignore, this script, or the bundle
# output path. Each of these files MUST be present in the deployment image —
# if any is missing the API server cannot spawn the Python strategy engine
# (Task #71) or cannot start at all.
REQUIRED_FILES=(
  ".pythonlibs/bin/python3"
  "python-strategy/main.py"
  "artifacts/api-server/dist/index.mjs"
  "artifacts/dashboard/dist/public/index.html"
)

MISSING=()
for f in "${REQUIRED_FILES[@]}"; do
  if [ ! -e "$f" ]; then
    MISSING+=("$f")
  fi
done

if [ ! -x ".pythonlibs/bin/python3" ] && [ -e ".pythonlibs/bin/python3" ]; then
  echo "ERROR: .pythonlibs/bin/python3 exists but is not executable" >&2
  exit 1
fi

if [ ${#MISSING[@]} -gt 0 ]; then
  echo "ERROR: Production deploy preflight failed — missing required files:" >&2
  for f in "${MISSING[@]}"; do
    echo "  - $f" >&2
  done
  echo "" >&2
  echo "If you changed .replitignore, scripts/build-production.sh, or the" >&2
  echo "api-server build output path, update them so the files above are" >&2
  echo "present after a clean build." >&2
  exit 1
fi

echo "==> Build complete."
