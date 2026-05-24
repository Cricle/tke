#!/usr/bin/env bash
set -euo pipefail
exec </dev/null

ROOT="${1:-/root/github/tke}"
OUT_DIR="${OUT_DIR:-$ROOT/.tmp-claude-e2e}"
PROMPT_DIR="${PROMPT_DIR:-/tmp/tke-claude-real-prompts}"

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

cat >"$PROMPT_DIR/diffcase.txt" <<'EOF'
Use bash exactly once. Run: git diff -- src/lib.rs.
Then answer with exactly three lines and nothing else:
STAGE=<selected command>
FILE=<dominant file path in the diff>
KIND=<short label for the output kind>
EOF

bash "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" raw findcase "$PROMPT_DIR/findcase.txt"
bash "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" rtk-hook findcase "$PROMPT_DIR/findcase.txt"

CLAUDE_TKE_LIVE_TOOLS=1 bash "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" tke livefind "$PROMPT_DIR/findcase.txt"
CLAUDE_TKE_LIVE_TOOLS=1 bash "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" tke livebuild "$PROMPT_DIR/buildcase.txt"
CLAUDE_TKE_LIVE_TOOLS=1 bash "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" tke liverg "$PROMPT_DIR/rgcase.txt"
CLAUDE_TKE_LIVE_TOOLS=1 bash "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" tke livediff "$PROMPT_DIR/diffcase.txt"

"$ROOT/target/release/tke" compare-e2e --agent claude --source "$OUT_DIR"
