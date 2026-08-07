#!/usr/bin/env bash
#
# Every check that has to pass before a commit, in one command that cannot lie.
#
# It exists because of a specific mistake. Summarising `cargo test` by hand — piping
# its output through `grep '^test result' | awk '{sum += $4}'` — silently counts the
# passing tests of a FAILING target, because "test result: FAILED. 205 passed;"
# starts with the same words and still has the passed count in the same column. A
# broken suite reads as a slightly lower total, and five failing tests were committed
# and pushed on the strength of that number.
#
# So the rule is: never summarise a test run. Let the tools set the exit status and
# let the shell propagate it.

set -euo pipefail

cd "$(dirname "$0")/.."

step() { printf '\n\033[1m── %s\033[0m\n' "$1"; }

step "task counts"
python3 tools/task_counts.py --check

step "rustfmt"
(cd src-tauri && cargo fmt --all --check)

step "clippy"
(cd src-tauri && cargo clippy --all-targets -- -D warnings)

step "cargo test"
# No pipe. `set -e` plus cargo's exit status is the whole mechanism, and anything
# between them is a chance to misread the result.
(cd src-tauri && cargo test)

step "svelte-check"
npm run check

step "frontend build"
npm run build

printf '\n\033[32mAll checks passed.\033[0m\n'
