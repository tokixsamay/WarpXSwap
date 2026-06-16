#!/usr/bin/env bash
# ci-check-replace-after-deploy.sh
#
# Bug #12 fix: CI guard that fails the build if any REPLACE_AFTER_DEPLOY
# placeholder strings remain in Rust source files or TypeScript.
#
# These sentinels are used during development to mark addresses that must be
# swapped for real deployed program IDs before going to mainnet.  Shipping with
# them in place (e.g. in constants.rs) means the on-chain program would try to
# CPI into a non-existent or wrong address, causing silent misbehaviour or
# immediate transaction failure.
#
# Usage (in CI pipeline):
#   bash scripts/ci-check-replace-after-deploy.sh
#
# Returns:
#   0 — no placeholders found (safe to proceed)
#   1 — one or more placeholders found (fail the build)

set -euo pipefail

PLACEHOLDER="REPLACE_AFTER_DEPLOY"

# Directories to scan — include all program source and TypeScript
SCAN_DIRS=(
  "WarpXSwap/programs"
  "WarpXSwap/sdk/src"
  "WarpXSwap/crank/src"
)

# File patterns to check
PATTERNS=("*.rs" "*.ts" "*.tsx" "*.js" "*.toml")

echo "=== Checking for '$PLACEHOLDER' in source files ==="

FOUND=0

for dir in "${SCAN_DIRS[@]}"; do
  if [[ ! -d "$dir" ]]; then
    echo "  [SKIP] $dir (directory not found)"
    continue
  fi

  for pattern in "${PATTERNS[@]}"; do
    # Use grep with -r (recursive), -l (files-only), -n (line number) for details
    while IFS= read -r -d '' file; do
      matches=$(grep -n "$PLACEHOLDER" "$file" 2>/dev/null || true)
      if [[ -n "$matches" ]]; then
        echo ""
        echo "  [ERROR] Found '$PLACEHOLDER' in: $file"
        echo "$matches" | while IFS= read -r line; do
          echo "          $line"
        done
        FOUND=$((FOUND + 1))
      fi
    done < <(find "$dir" -name "$pattern" -print0 2>/dev/null)
  done
done

echo ""
if [[ $FOUND -eq 0 ]]; then
  echo "=== PASS: No '$PLACEHOLDER' found in $FOUND files. ==="
  exit 0
else
  echo "=== FAIL: Found $FOUND file(s) with '$PLACEHOLDER'. ==="
  echo "    Replace all occurrences with real deployed program/account IDs"
  echo "    before merging to mainnet."
  exit 1
fi
