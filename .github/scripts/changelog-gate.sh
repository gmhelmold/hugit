#!/usr/bin/env bash
# changelog-gate.sh BASE HEAD
# If any commit subject in BASE..HEAD matches ^(feat|fix)[(:!], then:
#   (a) CHANGELOG.md must appear in git diff --name-only BASE..HEAD
#   (b) The ## [Unreleased] section of CHANGELOG.md must be non-empty
# Otherwise exits 0.

set -euo pipefail

if [ $# -ne 2 ]; then
  echo "Usage: changelog-gate.sh BASE HEAD" >&2
  exit 1
fi

BASE="$1"
HEAD="$2"

# Check if any feat: or fix: commits exist in the range
feat_fix_commits=$(git log --format="%s" "${BASE}..${HEAD}" 2>/dev/null | grep -E '^(feat|fix)[(:!]' || true)

if [ -z "$feat_fix_commits" ]; then
  # No feat/fix commits — no changelog requirement
  exit 0
fi

echo "Found feat/fix commit(s) — enforcing changelog discipline:"
echo "$feat_fix_commits"

# (a) CHANGELOG.md must be among changed files
changed_files=$(git diff --name-only "${BASE}..${HEAD}" 2>/dev/null || true)
if ! echo "$changed_files" | grep -q '^CHANGELOG\.md$'; then
  echo "ERROR: feat/fix commit(s) present but CHANGELOG.md was not updated." >&2
  echo "Please add an entry under ## [Unreleased] in CHANGELOG.md." >&2
  exit 1
fi

# (b) The ## [Unreleased] section must be non-empty
if [ ! -f CHANGELOG.md ]; then
  echo "ERROR: CHANGELOG.md does not exist." >&2
  exit 1
fi

# Extract content between ## [Unreleased] and the next ## heading
unreleased_content=$(awk '/^## \[Unreleased\]/{found=1; next} found && /^## /{exit} found{print}' CHANGELOG.md | grep -v '^[[:space:]]*$' || true)

if [ -z "$unreleased_content" ]; then
  echo "ERROR: CHANGELOG.md has an empty ## [Unreleased] section." >&2
  echo "Please add at least one entry describing your feat/fix changes." >&2
  exit 1
fi

echo "Changelog gate passed: ## [Unreleased] section is non-empty."
exit 0
