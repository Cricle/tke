#!/usr/bin/env bash
set -euo pipefail
exec </dev/null

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd -- "$SCRIPT_DIR/.." && pwd)}"
MODE="${2:-raw}"
NAME="${3:-findcase}"
PROMPT_FILE="${4:-}"

OUT_DIR="${OUT_DIR:-$ROOT/.tmp-codex-e2e-fair}"
WORK_REPO="${WORK_REPO:-/tmp/tke-codex-fair-repo}"
RTK_RULE_DIR="${RTK_RULE_DIR:-/tmp/tke-rtk-codex-proj}"

mkdir -p "$OUT_DIR"

if [[ -z "$PROMPT_FILE" || ! -f "$PROMPT_FILE" ]]; then
  echo "usage: $0 [root] [raw|rtk-codex-rules] [name] /abs/path/to/prompt.txt" >&2
  exit 2
fi

rm -rf "$WORK_REPO"
cp -a "$ROOT" "$WORK_REPO"
rm -f "$WORK_REPO/AGENTS.md" "$WORK_REPO/RTK.md"

if [[ "$MODE" == "rtk-codex-rules" ]]; then
  mkdir -p "$RTK_RULE_DIR"
  cat >"$RTK_RULE_DIR/AGENTS.md" <<'EOF'
@RTK.md
EOF
  cat >"$RTK_RULE_DIR/RTK.md" <<'EOF'
# RTK - Rust Token Killer (Codex CLI)

**Usage**: Token-optimized CLI proxy for shell commands.

## Rule

Always prefix shell commands with `rtk`.

Examples:

```bash
rtk git status
rtk cargo test
rtk npm run build
rtk pytest -q
```

## Meta Commands

```bash
rtk gain            # Token savings analytics
rtk gain --history  # Recent command savings history
rtk proxy <cmd>     # Run raw command without filtering
```

## Verification

```bash
rtk --version
rtk gain
which rtk
```
EOF
  cp "$RTK_RULE_DIR/AGENTS.md" "$WORK_REPO/AGENTS.md"
  cp "$RTK_RULE_DIR/RTK.md" "$WORK_REPO/RTK.md"
fi

JSON_OUT="$OUT_DIR/${NAME}.${MODE}.jsonl"
MSG_OUT="$OUT_DIR/${NAME}.${MODE}.txt"

timeout 240s codex exec \
  --ephemeral \
  --json \
  --dangerously-bypass-approvals-and-sandbox \
  -C "$WORK_REPO" \
  -o "$MSG_OUT" \
  "$(cat "$PROMPT_FILE")" </dev/null >"$JSON_OUT"
