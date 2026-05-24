#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-/root/github/tke}"
OUT_DIR="${OUT_DIR:-$ROOT/.tmp-codex-e2e}"
PROMPT_DIR="${PROMPT_DIR:-/tmp/tke-codex-real-prompts}"

mkdir -p "$PROMPT_DIR" "$OUT_DIR"

cat >"$PROMPT_DIR/findcase.txt" <<'EOF'
Use bash exactly once. Run: find src -type f | head -n 40. Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<first output path>
COUNT=<number of lines returned by the command>
EOF

cat >"$PROMPT_DIR/buildcase.txt" <<'EOF'
Use bash exactly once. Run: cargo test --lib -- --nocapture | tail -n 80. Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<dominant Rust file path mentioned in the output or src/lib.rs if the output is only test results>
COUNT=<number of failed tests shown in the output, or 0 if none>
EOF

cat >"$PROMPT_DIR/rgcase.txt" <<'EOF'
Use bash exactly once. Run: rg -n "assert|benchmark|normalize|claude" src/tests.rs.
Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<dominant file path in the output>
KIND=<short label for the output kind>
EOF

cat >"$PROMPT_DIR/realtask.txt" <<'EOF'
Use bash exactly once. Run: cat src/benchmark.rs | rg -n "BenchmarkTaskReport|benchmark_specs|benchmark_task_specs|BenchmarkSpec" | head -n 40.
Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<dominant file path in the output>
COUNT=<number of lines returned by the command>
EOF

for mode in raw tke; do
  bash "$ROOT/scripts/codex_e2e_compare.sh" "$ROOT" "$mode" findcase "$PROMPT_DIR/findcase.txt"
  bash "$ROOT/scripts/codex_e2e_compare.sh" "$ROOT" "$mode" buildcase "$PROMPT_DIR/buildcase.txt"
  bash "$ROOT/scripts/codex_e2e_compare.sh" "$ROOT" "$mode" rgcase "$PROMPT_DIR/rgcase.txt"
  bash "$ROOT/scripts/codex_e2e_compare.sh" "$ROOT" "$mode" realtask "$PROMPT_DIR/realtask.txt"
done

"$ROOT/target/release/tke" compare-e2e --agent codex --source "$OUT_DIR"
