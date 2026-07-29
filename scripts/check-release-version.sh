#!/usr/bin/env bash
# Release guard: a version tag must name the version the binaries were built from.
#
# The manifest is the single source of truth — `[workspace.package] version` in the root Cargo.toml,
# inherited by every crate and compiled into the binaries as CARGO_PKG_VERSION. A git tag only names
# a commit, so it is checked here rather than injected into the build: `rpt --version` then says the
# same thing whether the binary came from a release, a checkout or `cargo install`.
#
# Usage: scripts/check-release-version.sh [vX.Y.Z]   (defaults to $GITHUB_REF_NAME in CI)
set -euo pipefail

cd "$(dirname "$0")/.."

tag="${1:-${GITHUB_REF_NAME:-}}"
if [[ -z $tag ]]; then
  echo "usage: scripts/check-release-version.sh vX.Y.Z" >&2
  exit 2
fi

# The version from the [workspace.package] table, ignoring a `version` key in any other table.
manifest=$(awk '
  /^\[/ { in_table = ($0 == "[workspace.package]"); next }
  in_table && /^version[[:space:]]*=/ {
    match($0, /"[^"]*"/); print substr($0, RSTART + 1, RLENGTH - 2); exit
  }
' Cargo.toml)

if [[ -z $manifest ]]; then
  echo "error: no version in [workspace.package] of Cargo.toml" >&2
  exit 1
fi

fail=0

if [[ $tag != "v$manifest" ]]; then
  cat >&2 <<EOF
error: release tag does not match the manifest version
  tag:      $tag
  manifest: $manifest  ([workspace.package] version in Cargo.toml)

Tag the release as 'v$manifest', or bump [workspace.package] version to match the tag and re-tag.
EOF
  fail=1
fi

# The single source only holds if every member inherits it; a literal version in a member manifest
# would ship one binary reporting something else while this check still passed.
for m in crates/*/Cargo.toml apps/*/Cargo.toml; do
  if ! grep -q '^version\.workspace = true$' "$m"; then
    echo "error: $m does not inherit the workspace version (want 'version.workspace = true')" >&2
    fail=1
  fi
done

if ((fail)); then
  exit 1
fi

echo "release version ok: tag $tag matches [workspace.package] version $manifest"
