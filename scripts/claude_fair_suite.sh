#!/usr/bin/env bash
set -euo pipefail
exec </dev/null

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd -- "$SCRIPT_DIR/.." && pwd)}"
OUT_DIR="${OUT_DIR:-$ROOT/.tmp-claude-e2e-fair}"
PROMPT_DIR="${PROMPT_DIR:-/tmp/tke-claude-fair-prompts}"
MAX_ATTEMPTS="${MAX_ATTEMPTS:-3}"
RETRY_SLEEP_SECS="${RETRY_SLEEP_SECS:-5}"

mkdir -p "$PROMPT_DIR" "$OUT_DIR"

cat >"$PROMPT_DIR/fairfind.txt" <<'EOF'
Use bash exactly once. Run: rg --files src | head -n 40. Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<first output path>
COUNT=<number of lines returned by the command>
EOF

cat >"$PROMPT_DIR/fairrg.txt" <<'EOF'
Use bash exactly once. Run: rg -n "normalize_text|rewrite_agent_transcript|compare-e2e|benchmark-commands" src
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

run_case() {
  local mode="$1"
  local name="$2"
  local prompt="$3"
  local stream_out="$OUT_DIR/${name}.${mode}.stream.jsonl"
  local attempt_json="$OUT_DIR/${name}.${mode}.attempt.json"
  local attempt

  rm -f "$stream_out"
  for ((attempt = 1; attempt <= MAX_ATTEMPTS; attempt++)); do
    OUT_DIR="$OUT_DIR" bash "$ROOT/scripts/claude_smoke_try.sh" "$ROOT" "$mode" "$name" "$prompt" || true
    if [[ -f "$stream_out" ]]; then
      return 0
    fi
    if [[ -f "$attempt_json" ]]; then
      echo "retrying $name/$mode after unsuccessful attempt $attempt" >&2
    fi
    if (( attempt < MAX_ATTEMPTS )); then
      sleep "$RETRY_SLEEP_SECS"
    fi
  done
  return 0
}

for mode in raw rtk-hook; do
  run_case "$mode" fairfind "$PROMPT_DIR/fairfind.txt"
  run_case "$mode" fairrg "$PROMPT_DIR/fairrg.txt"
  run_case "$mode" fairbuild "$PROMPT_DIR/fairbuild.txt"
done

"$ROOT/target/release/tke" compare-e2e --agent claude --source "$OUT_DIR"
