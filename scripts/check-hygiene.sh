#!/usr/bin/env bash
# Fails if tracked files contain personal or machine-specific content.
# Public-repo rule: AGENTS.md §3.1. Run locally: bash scripts/check-hygiene.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

fail=0
exclude=(':!scripts/check-hygiene.sh')

check() {
  local label="$1" pattern="$2"
  local hits
  hits=$(git grep -I -n -E "$pattern" -- . "${exclude[@]}" || true)
  if [[ -n "$hits" ]]; then
    echo "::error::$label"
    echo "$hits"
    fail=1
  fi
}

# Absolute home-folder paths (Windows, macOS, Linux)
check "Absolute user path found" '[A-Za-z]:[\\/]+Users[\\/]+[A-Za-z0-9._-]+|/Users/[A-Za-z0-9._-]+/|/home/[A-Za-z0-9._-]+/'
# OneDrive personal folders
check "OneDrive path found" 'OneDrive( -|[\\/])'
# Likely secrets
check "Possible secret found" '(api[_-]?key|secret|token)[[:space:]]*[:=][[:space:]]*["'"'"'][A-Za-z0-9_\-]{16,}'
check "Private key found" 'BEGIN [A-Z ]*PRIVATE KEY'

# Email addresses, except placeholders and noreply
emails=$(git grep -I -n -o -E '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' -- . "${exclude[@]}" \
  | grep -v -E '@(users\.)?noreply\.github\.com|@example\.(com|org|net)' || true)
if [[ -n "$emails" ]]; then
  echo "::error::Email address found"
  echo "$emails"
  fail=1
fi

if [[ $fail -ne 0 ]]; then
  echo "Hygiene check failed: see AGENTS.md §3.1"
  exit 1
fi
echo "Hygiene check passed"
