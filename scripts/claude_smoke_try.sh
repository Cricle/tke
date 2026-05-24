#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-/root/github/tke}"
MODE="${2:-raw}"
NAME="${3:-smoke}"
PROMPT_FILE="${4:-}"

OUT_DIR="${OUT_DIR:-$ROOT/.tmp-claude-e2e}"
CLAUDE_BASE_URL="${CLAUDE_BASE_URL:-https://ai.fixwikihub.com/}"
CLAUDE_API_KEY="${CLAUDE_API_KEY:-}"
CLAUDE_MODEL="${CLAUDE_MODEL:-claude-sonnet-4-5}"

mkdir -p "$OUT_DIR"

if [[ -z "$PROMPT_FILE" || ! -f "$PROMPT_FILE" ]]; then
  echo "usage: $0 [root] [raw|tke|rtk-hook] [name] /abs/path/to/prompt.txt" >&2
  exit 2
fi

if [[ -z "$CLAUDE_API_KEY" ]]; then
  echo "CLAUDE_API_KEY is required" >&2
  exit 2
fi

set +e
timeout 240s "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" "$MODE" "$NAME" "$PROMPT_FILE"
RC=$?
set -e

STATUS_JSON="$OUT_DIR/${NAME}.${MODE}.attempt.json"
STREAM_OUT="$OUT_DIR/${NAME}.${MODE}.stream.jsonl"
DEBUG_OUT="$OUT_DIR/${NAME}.${MODE}.debug.log"

python - "$RC" "$MODE" "$NAME" "$CLAUDE_MODEL" "$STREAM_OUT" "$DEBUG_OUT" "$STATUS_JSON" <<'PY'
import json, pathlib, re, sys

rc = int(sys.argv[1])
mode = sys.argv[2]
name = sys.argv[3]
model = sys.argv[4]
stream_path = pathlib.Path(sys.argv[5])
debug_path = pathlib.Path(sys.argv[6])
status_path = pathlib.Path(sys.argv[7])

result = {
    "v": 1,
    "name": name,
    "mode": mode,
    "model": model,
    "exit_code": rc,
    "ok": False,
    "completed": False,
    "result_is_error": False,
    "api_retry_count": 0,
    "error_statuses": [],
    "notes": [],
}

if stream_path.exists():
    for line in stream_path.read_text(errors="ignore").splitlines():
        try:
            obj = json.loads(line)
        except Exception:
            continue
        if obj.get("type") == "assistant":
            result["ok"] = True
        if obj.get("type") == "system" and obj.get("subtype") == "api_retry":
            result["api_retry_count"] += 1
            status = obj.get("error_status")
            if status is not None:
                result["error_statuses"].append(status)
        if obj.get("type") == "result":
            result["completed"] = True
            if obj.get("is_error") is True:
                result["result_is_error"] = True
                status = obj.get("api_error_status")
                if status is not None:
                    result["error_statuses"].append(status)

if debug_path.exists():
    text = debug_path.read_text(errors="ignore")
    for match in re.finditer(r"No available accounts: ([^\n]+)", text):
        result["notes"].append(match.group(1).strip())
        break

result["error_statuses"] = sorted(set(result["error_statuses"]))
result["ok"] = (
    rc == 0
    and result["completed"]
    and not result["result_is_error"]
    and not result["error_statuses"]
)

status_path.write_text(json.dumps(result, ensure_ascii=True))
print(json.dumps(result, ensure_ascii=True))
PY
