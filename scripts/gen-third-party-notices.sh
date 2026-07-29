#!/usr/bin/env bash
# Regenerate THIRD-PARTY-NOTICES.md — the notice file shipped inside every release archive and the
# Docker image.
#
# The binaries are statically linked and embed the bundled fonts, so a published archive contains a
# substantial portion of every dependency and every font face. MIT, Apache-2.0 §4, the SIL OFL and
# the Bitstream Vera licence each condition that redistribution on carrying their notice, and the
# project's own LICENSE covers none of them. This file is that notice.
#
# Two parts, because they have different sources: the dependency tree comes from the lock file via
# `cargo about` (config in about.toml), rendered by scripts/third-party-notices.py, and the fonts are
# vendored bytes cargo knows nothing about, so their texts are appended from crates/rpt-text/fonts/.
#
# Usage:
#   scripts/gen-third-party-notices.sh            # rewrite THIRD-PARTY-NOTICES.md
#   scripts/gen-third-party-notices.sh --check    # fail if the committed file is out of date
set -euo pipefail

cd "$(dirname "$0")/.."

check=0
if [[ ${1:-} == --check ]]; then
  check=1
elif [[ $# -gt 0 ]]; then
  echo "usage: scripts/gen-third-party-notices.sh [--check]" >&2
  exit 2
fi

if ! cargo about --version >/dev/null 2>&1; then
  cat >&2 <<'EOF'
error: cargo-about is not installed
  cargo install cargo-about --locked --features cli
EOF
  exit 1
fi

out="THIRD-PARTY-NOTICES.md"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/notices.md" <<'EOF'
# Third-party notices

The `rpt` and `rpt-render` binaries are statically linked, so the crates listed here are compiled
into them and distributed with them, and the bundled font faces are embedded in the binary itself.
Their licences are reproduced below to satisfy the notice conditions those licences attach to
redistribution.

This project's own licence is `LICENSE` (MPL-2.0); the workspace's own crates appear below as well,
since they are part of the same linked image.

Each section lists every crate under a licence with the copyright line its own licence file states —
that line, not the shared boilerplate, is what these licences condition redistribution on — and then
reproduces the terms once, verbatim from one crate's copy. Copies are collapsed only where they say
the same thing: crates typeset the same licence at different widths, with different titles and with
or without its appendix. Where a copy's terms genuinely differ it appears as its own variant. A crate
listed without a copyright line ships a licence file that carries none.

Regenerate with `scripts/gen-third-party-notices.sh`.

EOF

# --offline: crawl only the licence files already in the registry cache. Left online, cargo-about
# falls back to clearlydefined.io for anything it cannot resolve locally, which makes the output
# depend on a third-party service and stops `--check` from being reproducible. Run `cargo fetch
# --locked` first if a dependency is not in the cache yet.
#
# Some upstream licence texts are stored with CRLF; stripping the CRs keeps the generated file
# LF-only, so git's end-of-line normalisation cannot make a freshly checked-out copy differ from
# what this script produces and fail `--check`.
# cargo metadata resolves each crate to its unpacked source, which is where an upstream NOTICE file
# lives — cargo-about reports licence files and nothing else.
cargo metadata --locked --offline --format-version 1 > "$tmp/metadata.json"

cargo about generate --workspace --fail --locked --offline --format json \
  | python3 scripts/third-party-notices.py "$tmp/metadata.json" \
  | tr -d '\r' >> "$tmp/notices.md"

{
  printf '\n# Bundled fonts\n\n'
  printf 'The faces below are embedded in the binaries (`rpt_text::FontProvider::bundled`) and are\n'
  printf 'not cargo dependencies, so their licences are reproduced here in full.\n'

  printf '\n## Liberation fonts (SIL Open Font License 1.1)\n\n'
  printf 'LiberationSans, LiberationSerif and LiberationMono, regular/bold/italic as bundled.\n\n'
  printf '```\n'
  cat crates/rpt-text/fonts/LICENSE
  printf '```\n'

  printf '\n## DejaVu Sans (Bitstream Vera licence)\n\n'
  printf 'Bundled as the symbol fallback face.\n\n'
  printf '```\n'
  cat crates/rpt-text/fonts/LICENSE-DejaVu
  printf '```\n'
} >> "$tmp/notices.md"

if (( check )); then
  if ! diff -u "$out" "$tmp/notices.md" >/dev/null 2>&1; then
    echo "error: $out is out of date — run scripts/gen-third-party-notices.sh" >&2
    diff -u "$out" "$tmp/notices.md" >&2 || true
    exit 1
  fi
  echo "$out is up to date"
else
  mv "$tmp/notices.md" "$out"
  echo "wrote $out"
fi
