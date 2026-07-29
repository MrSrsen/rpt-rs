#!/usr/bin/env bash
# Drift guard: every dependency declared in the root [workspace.dependencies] must be inherited by
# member crates via `{ workspace = true }`, never re-declared with a bare version. A bare per-crate
# version can silently drift into a second compiled copy (the classic case being `fontdb`, shared by
# the pdf and text crates). Run from the workspace root; exits non-zero on any violation.
set -euo pipefail

cd "$(dirname "$0")/.."

# Names declared in the root [workspace.dependencies] table.
names=$(awk '
  /^\[workspace\.dependencies\]/ { in_table = 1; next }
  /^\[/                          { in_table = 0 }
  in_table && /^[A-Za-z0-9_-]+[[:space:]]*=/ { print $1 }
' Cargo.toml)

# An empty name list makes every check below a no-op, so the guard would report OK on a workspace
# that violates it — the failure mode is silent and total. Absent or renamed, the table is a
# structural change this script must not paper over.
if [ -z "$names" ]; then
    echo "check-workspace-deps: [workspace.dependencies] is empty or unreadable; nothing was checked." >&2
    exit 2
fi

fail=0
for manifest in crates/*/Cargo.toml apps/*/Cargo.toml; do
  for name in $names; do
    # The leaf's declaration line for this dep, if any. `workspace = true` is the only allowed form.
    line=$(grep -E "^${name}[[:space:]]*=" "$manifest" || true)
    [ -z "$line" ] && continue
    case "$line" in
      *"workspace = true"*) : ;;
      *)
        echo "::error file=${manifest}::'${name}' is in [workspace.dependencies] but re-declared with a bare version here; use { workspace = true } (features/optional stay at the leaf)"
        fail=1
        ;;
    esac
  done
done

if [ "$fail" -ne 0 ]; then
  echo "workspace-dependency drift check FAILED" >&2
  exit 1
fi
echo "workspace-dependency drift check OK"
