#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-/root/github/tke}"
MODE="${2:-raw}"
NAME="${3:-smoke}"
PROMPT_FILE="${4:-}"

HOST_TKE_BIN="${TKE_BIN:-$ROOT/target/release/tke}"
HOST_RTK_BIN="${RTK_BIN:-/tmp/rtk-bin/rtk}"
HOST_CLAUDE_BIN="${CLAUDE_BIN:-$(command -v claude)}"
HOST_CLAUDE_HOME="${HOST_CLAUDE_HOME:-/root/.claude}"
HOST_CLAUDE_STATE="${HOST_CLAUDE_STATE:-/root/.claude.json}"

OUT_DIR="${OUT_DIR:-$ROOT/.tmp-claude-e2e}"
WORK_ROOT="${WORK_ROOT:-/tmp/tke-claude-e2e}"
RUN_AS_USER="${RUN_AS_USER:-nobody}"
RUN_AS_GROUP="${RUN_AS_GROUP:-nogroup}"
CLAUDE_BASE_URL="${CLAUDE_BASE_URL:-https://ai.fixwikihub.com}"
CLAUDE_API_KEY="${CLAUDE_API_KEY:-}"
CLAUDE_MODEL="${CLAUDE_MODEL:-claude-sonnet-4-5}"

if [[ -z "$PROMPT_FILE" || ! -f "$PROMPT_FILE" ]]; then
  echo "usage: $0 [root] [raw|wrapped|tke|rtk|rtk-hook] [name] /abs/path/to/prompt.txt" >&2
  exit 2
fi

if [[ -z "$CLAUDE_API_KEY" ]]; then
  echo "CLAUDE_API_KEY is required" >&2
  exit 2
fi

mkdir -p "$OUT_DIR" "$WORK_ROOT"

RUN_ROOT="$WORK_ROOT/$MODE"
REPO_ROOT="$RUN_ROOT/repo"
HOME_ROOT="$RUN_ROOT/home"
BIN_ROOT="$RUN_ROOT/bin"
SHIM_DIR="$RUN_ROOT/shims"

rm -rf "$RUN_ROOT"
mkdir -p "$REPO_ROOT" "$HOME_ROOT/.claude" "$BIN_ROOT"

cp "$HOST_TKE_BIN" "$BIN_ROOT/tke"
if [[ -x "$HOST_RTK_BIN" ]]; then
  cp "$HOST_RTK_BIN" "$BIN_ROOT/rtk"
  chmod +x "$BIN_ROOT/rtk"
fi
cp "$HOST_CLAUDE_BIN" "$BIN_ROOT/claude"
chmod +x "$BIN_ROOT/tke" "$BIN_ROOT/claude"

mkdir -p "$REPO_ROOT/src" "$REPO_ROOT/scripts" "$REPO_ROOT/.github/workflows"
cp -a "$ROOT/src/." "$REPO_ROOT/src/"
for f in Cargo.toml Cargo.lock README.md .gitignore; do
  if [[ -f "$ROOT/$f" ]]; then
    cp "$ROOT/$f" "$REPO_ROOT/$f"
  fi
done
if [[ -d "$ROOT/scripts" ]]; then
  find "$ROOT/scripts" -maxdepth 1 -type f -name '*.sh' -exec cp {} "$REPO_ROOT/scripts/" \;
fi
if [[ -d "$ROOT/.github/workflows" ]]; then
  find "$ROOT/.github/workflows" -maxdepth 1 -type f -exec cp {} "$REPO_ROOT/.github/workflows/" \;
fi

if [[ -f "$HOST_CLAUDE_STATE" ]]; then
  cp "$HOST_CLAUDE_STATE" "$HOME_ROOT/.claude.json"
fi
if [[ -d "$HOST_CLAUDE_HOME" ]]; then
  cp -a "$HOST_CLAUDE_HOME/." "$HOME_ROOT/.claude/"
fi

PROMPT_COPY="$RUN_ROOT/prompt.txt"
cp "$PROMPT_FILE" "$PROMPT_COPY"

SETTINGS_FILE="$RUN_ROOT/settings.json"
cat >"$SETTINGS_FILE" <<JSON
{
  "env": {
    "ANTHROPIC_BASE_URL": "${CLAUDE_BASE_URL}",
    "ANTHROPIC_API_KEY": "${CLAUDE_API_KEY}",
    "ANTHROPIC_AUTH_TOKEN": "${CLAUDE_API_KEY}",
    "ANTHROPIC_MODEL": "${CLAUDE_MODEL}"
  }
}
JSON

RAW_STREAM_OUT="$OUT_DIR/${NAME}.${MODE}.stream.jsonl"
RAW_TEXT_OUT="$OUT_DIR/${NAME}.${MODE}.txt"
RAW_DEBUG_OUT="$OUT_DIR/${NAME}.${MODE}.debug.log"
RAW_STATUS_OUT="$OUT_DIR/${NAME}.${MODE}.status"
RAW_SESSION_OUT="$OUT_DIR/${NAME}.${MODE}.session"

TMP_STREAM_OUT="$RUN_ROOT/out.stream.jsonl"
TMP_TEXT_OUT="$RUN_ROOT/out.txt"
TMP_DEBUG_OUT="$RUN_ROOT/debug.log"
TMP_STATUS_OUT="$RUN_ROOT/status"
TMP_SESSION_OUT="$RUN_ROOT/session"

RUN_SCRIPT="$RUN_ROOT/run.sh"
cat >"$RUN_SCRIPT" <<'SH'
#!/usr/bin/env bash
set -euo pipefail

MODE="$1"
REPO_ROOT="$2"
HOME_ROOT="$3"
BIN_ROOT="$4"
SHIM_DIR="$5"
SETTINGS_FILE="$6"
PROMPT_COPY="$7"
RAW_STREAM_OUT="$8"
RAW_TEXT_OUT="$9"
RAW_DEBUG_OUT="${10}"
RAW_STATUS_OUT="${11}"
RAW_SESSION_OUT="${12}"
CLAUDE_BASE_URL="${13}"
CLAUDE_API_KEY="${14}"
CLAUDE_MODEL="${15}"

export HOME="$HOME_ROOT"
export PATH="$BIN_ROOT:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export ANTHROPIC_BASE_URL="$CLAUDE_BASE_URL"
export ANTHROPIC_API_KEY="$CLAUDE_API_KEY"
export ANTHROPIC_AUTH_TOKEN="$CLAUDE_API_KEY"
export ANTHROPIC_MODEL="$CLAUDE_MODEL"
export CLAUDE_CODE_SIMPLE=1

if [[ "$MODE" == "wrapped" || "$MODE" == "tke" ]]; then
  eval "$("$BIN_ROOT/tke" activate --shim-dir "$SHIM_DIR" claude)"
fi

if [[ "$MODE" == "rtk" || "$MODE" == "rtk-hook" ]]; then
  mkdir -p "$HOME_ROOT/.claude" "$HOME_ROOT/.config"
  HOME="$HOME_ROOT" XDG_CONFIG_HOME="$HOME_ROOT/.config" "$BIN_ROOT/rtk" init -g --auto-patch --agent claude >/dev/null
fi

CLAUDE_LAUNCH="$BIN_ROOT/claude"
if [[ "$MODE" == "wrapped" || "$MODE" == "tke" || "$MODE" == "rtk" || "$MODE" == "rtk-hook" ]]; then
  CLAUDE_LAUNCH="claude"
fi

cd "$REPO_ROOT"

set +e
cat "$PROMPT_COPY" | "$CLAUDE_LAUNCH" -p \
  --input-format text \
  --output-format stream-json \
  --verbose \
  --bare \
  --permission-mode acceptEdits \
  --allowedTools "Bash" \
  --tools "Bash" \
  --model "$CLAUDE_MODEL" \
  --settings "$SETTINGS_FILE" \
  --debug-file "$RAW_DEBUG_OUT" \
  --add-dir "$REPO_ROOT" >"$RAW_STREAM_OUT" 2>"$RAW_TEXT_OUT"
STATUS=$?
set -e

printf '%s\n' "$STATUS" >"$RAW_STATUS_OUT"
LATEST_SESSION="$(find "$HOME_ROOT/.claude/sessions" -type f 2>/dev/null | sort | tail -n 1 || true)"
printf '%s\n' "$LATEST_SESSION" >"$RAW_SESSION_OUT"
exit 0
SH
chmod +x "$RUN_SCRIPT"

chown -R "$RUN_AS_USER:$RUN_AS_GROUP" "$RUN_ROOT"

runuser -u "$RUN_AS_USER" -- "$RUN_SCRIPT" \
  "$MODE" \
  "$REPO_ROOT" \
  "$HOME_ROOT" \
  "$BIN_ROOT" \
  "$SHIM_DIR" \
  "$SETTINGS_FILE" \
  "$PROMPT_COPY" \
  "$TMP_STREAM_OUT" \
  "$TMP_TEXT_OUT" \
  "$TMP_DEBUG_OUT" \
  "$TMP_STATUS_OUT" \
  "$TMP_SESSION_OUT" \
  "$CLAUDE_BASE_URL" \
  "$CLAUDE_API_KEY" \
  "$CLAUDE_MODEL"

cp -f "$TMP_STREAM_OUT" "$RAW_STREAM_OUT" 2>/dev/null || : 
cp -f "$TMP_TEXT_OUT" "$RAW_TEXT_OUT" 2>/dev/null || :
cp -f "$TMP_DEBUG_OUT" "$RAW_DEBUG_OUT" 2>/dev/null || :
cp -f "$TMP_STATUS_OUT" "$RAW_STATUS_OUT" 2>/dev/null || :
cp -f "$TMP_SESSION_OUT" "$RAW_SESSION_OUT" 2>/dev/null || :
