#!/usr/bin/env bash
# Guard the error-handling invariants clippy cannot express.
#
# Companion to check-workspace-deps.sh: a small, specific grep gate for patterns that are structurally
# invisible to the compiler. Each check below guards a defect that was actually shipped and fixed, so
# it earns its false-positive budget; a check that would fire on legitimate code is deliberately NOT
# here (see the notes at the bottom).
#
# Exits nonzero on the first violation, naming the file and what to do instead.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 2
status=0

# Production sources only. A test may legitimately ignore diagnostics — it is asserting on the AST.
sources() {
    find crates/*/src apps/*/src -name '*.rs' \
        -not -path '*/tests/*' -not -name 'tests.rs' -not -name '*_tests.rs' 2>/dev/null
}

# A guard that scans nothing passes everything. If the source set is empty the layout has moved
# under this script, and reporting OK would be a false all-clear rather than a result.
if [ "$(sources | wc -l)" -lt 20 ]; then
    echo "check-error-handling: found $(sources | wc -l) source file(s); the tree layout has moved and nothing meaningful was scanned." >&2
    exit 2
fi

fail() {
    printf '\n%s\n' "$1" >&2
    status=1
}

# 1. Discarded formula parse diagnostics.
#
# `parse()` returns (Node, Vec<Diagnostic>). Binding the diagnostics to `_` means a formula with a
# syntax error is compiled from the parser's partial recovery AST and evaluated anyway, producing a
# meaningless value with nothing reported — the field renders blank and the user cannot tell a broken
# formula from a null value. Every production call site must report them.
#
# A `#[cfg(test)]` module inside a production file is excluded by the `mod tests` boundary check
# below rather than by path, since inline test modules are common here.
discarded_parse=$(
    while IFS= read -r f; do
        # Stop at an inline `#[cfg(test)]` module: everything after it is test code.
        awk '/^#\[cfg\(test\)\]/ { exit } { print FILENAME ":" FNR ":" $0 }' "$f"
    done < <(sources) | grep -E 'let \([A-Za-z_]+, _\) = parse\(' || true
)
if [ -n "$discarded_parse" ]; then
    fail "error-handling: formula parse diagnostics discarded in production code.
A formula that does not parse is still compiled and evaluated, so its value is meaningless. Report
the diagnostics (rpt_data::diagnostics::report_parse_diagnostics, or rpt-layout's
parse_cached_reporting) instead of binding them to \`_\`:

$discarded_parse"
fi

# 2. An error variant that both interpolates its source AND marks it #[source]/#[from].
#
# The chain printer walks source(), so an interpolated source is printed twice:
#   "connection failed: error connecting to server: error connecting to server: Connection refused"
# The convention is that a variant carrying a source never interpolates it.
#
# Matched narrowly: `{0}` in the message with `#[from]` on the *next* line, which is the exact
# tuple-variant shape that caused this. `#[error("{0}")] Variant(String, #[source] E)` is fine and is
# not matched, because there `{0}` is the context string, not the source.
double_print=$(
    while IFS= read -r f; do
        grep -Pzo '#\[error\("[^"]*\{0\}[^"]*"\)\]\n\s*[A-Za-z_]+\(#\[(from|source)\]' "$f" \
            2>/dev/null | tr '\0' '\n' | sed "s|^|$f: |" || true
    done < <(sources) | grep -E '#\[error' || true
)
if [ -n "$double_print" ]; then
    fail "error-handling: an error variant interpolates the same value it marks as its source.
The cause is then printed twice by any chain-walking reporter (rpt_reader::error_chain). Drop the {0} from
the message and let the chain supply the cause:

$double_print"
fi

# NOT CHECKED, deliberately:
#
#   `.ok()?` — reads as "discard the error", but every current use is on an Option-returning helper
#   where the error carries nothing (`slice.try_into()`, `str::parse` used as a format probe). The
#   check would be all false positives, and a gate that is always wrong gets disabled.
#
#   `.unwrap()` / `.expect()` — covered by judgement per site (see the clippy block in Cargo.toml);
#   a grep cannot tell a proven invariant from an unchecked assumption.

if [ "$status" -eq 0 ]; then
    echo "error-handling check OK"
fi
exit "$status"
