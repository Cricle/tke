#!/usr/bin/env bash
set -euo pipefail
exec </dev/null

ROOT="${1:-/root/github/tke}"
OUT_DIR="${OUT_DIR:-$ROOT/.tmp-codex-e2e-fair}"
PROMPT_DIR="${PROMPT_DIR:-/tmp/tke-codex-fair-prompts}"

mkdir -p "$PROMPT_DIR" "$OUT_DIR"

cat >"$PROMPT_DIR/fairfind.txt" <<'EOF'
Use bash exactly once. Run: rg --files src | head -n 40. Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<first output path>
COUNT=<number of lines returned by the command>
EOF

cat >"$PROMPT_DIR/fairrg.txt" <<'EOF'
Use bash exactly once. Run: rg -n "normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands" src.
Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<dominant file path in the output>
KIND=<short label for the output kind>
EOF

cat >"$PROMPT_DIR/fairbuild.txt" <<'EOF'
Use bash exactly once. Run: cargo test --lib -- --nocapture | tail -n 80. Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<dominant Rust file path mentioned in the output or src/lib.rs if the output is only test results>
COUNT=<number of failed tests shown in the output, or 0 if none>
EOF

for mode in raw rtk-codex-rules; do
  bash "$ROOT/scripts/codex_fair_compare.sh" "$ROOT" "$mode" fairfind "$PROMPT_DIR/fairfind.txt"
  bash "$ROOT/scripts/codex_fair_compare.sh" "$ROOT" "$mode" fairrg "$PROMPT_DIR/fairrg.txt"
  bash "$ROOT/scripts/codex_fair_compare.sh" "$ROOT" "$mode" fairbuild "$PROMPT_DIR/fairbuild.txt"
done

"$ROOT/target/release/tke" compare-e2e --agent codex --source "$OUT_DIR"
