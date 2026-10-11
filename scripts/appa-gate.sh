#!/usr/bin/env bash
# The OpenAPPA policy gate for pawan.
#
# Exit contract (measured against the pinned rev):
#   0 — describe loads the complete configuration AND every pass trace matches
#   1 — a replay MISMATCH (the harness can fail; the negative set proves it)
#   2 — could not run (parse failure, unloadable config, unbound paths)
#
# The appa binary is built from the pinned source by the caller (CI job or
# local run): the auditable path is `cargo build` at the rev, never the
# upstream install.sh.
set -uuo pipefail

CONFIG="${1:-config/appa.test.toml}"
PASS_DIR="${2:-policy-tests}"
NEG_DIR="${3:-policy-tests-negative}"
APPA_BIN="${APPA_BIN:-appa}"

if ! command -v "$APPA_BIN" >/dev/null 2>&1; then
    echo "appa-gate: the appa binary is not on PATH; build it from the pinned rev first" >&2
    exit 2
fi

echo "== appa describe --check ($CONFIG)"
"$APPA_BIN" describe --config "$CONFIG" --check || exit $?

echo "== appa replay (pass set: $PASS_DIR)"
"$APPA_BIN" replay --config "$CONFIG" "$PASS_DIR" || exit $?

# The negative control: a trace asserting the opposite of the policy's
# answer MUST fail. A gate that cannot fail is not a gate.
echo "== appa replay (negative control: $NEG_DIR, rc=1 required)"
set +e
"$APPA_BIN" replay --config "$CONFIG" "$NEG_DIR"
NEG_RC=$?
set -e
if [ "$NEG_RC" -ne 1 ]; then
    echo "appa-gate: the negative control exited $NEG_RC, expected 1 — the harness cannot discriminate" >&2
    exit 2
fi
echo "appa-gate: negative control failed as required (rc=1)"

echo "appa-gate: PASS"
exit 0
