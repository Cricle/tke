#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd -- "$SCRIPT_DIR/.." && pwd)}"
MODE="${2:-raw}"
NAME="${3:-smoke}"
PROMPT_FILE="${4:-}"

OUT_DIR="${OUT_DIR:-$ROOT/.tmp-claude-e2e}"
CLAUDE_BASE_URL="${CLAUDE_BASE_URL:-https://ai.fixwikihub.com/}"
CLAUDE_API_KEY="${CLAUDE_API_KEY:-}"
CLAUDE_MODEL="${CLAUDE_MODEL:-claude-opus-4-6}"

mkdir -p "$OUT_DIR"

if [[ -z "$PROMPT_FILE" || ! -f "$PROMPT_FILE" ]]; then
  echo "usage: $0 [root] [raw|tke|rtk-hook] [name] /abs/path/to/prompt.txt" >&2
  exit 2
fi

if [[ -z "$CLAUDE_API_KEY" ]]; then
  echo "CLAUDE_API_KEY is required" >&2
  exit 2
fi

STATUS_JSON="$OUT_DIR/${NAME}.${MODE}.attempt.json"
STREAM_OUT="$OUT_DIR/${NAME}.${MODE}.stream.jsonl"
DEBUG_OUT="$OUT_DIR/${NAME}.${MODE}.debug.log"
FAILED_STREAM_OUT="$OUT_DIR/${NAME}.${MODE}.failed.stream.jsonl"
FAILED_DEBUG_OUT="$OUT_DIR/${NAME}.${MODE}.failed.debug.log"
TEXT_OUT="$OUT_DIR/${NAME}.${MODE}.txt"
STATUS_OUT="$OUT_DIR/${NAME}.${MODE}.status"
SESSION_OUT="$OUT_DIR/${NAME}.${MODE}.session"
FAILED_TEXT_OUT="$OUT_DIR/${NAME}.${MODE}.failed.txt"
FAILED_STATUS_OUT="$OUT_DIR/${NAME}.${MODE}.failed.status"
FAILED_SESSION_OUT="$OUT_DIR/${NAME}.${MODE}.failed.session"

rm -f \
  "$STATUS_JSON" \
  "$STREAM_OUT" \
  "$DEBUG_OUT" \
  "$TEXT_OUT" \
  "$STATUS_OUT" \
  "$SESSION_OUT" \
  "$FAILED_STREAM_OUT" \
  "$FAILED_DEBUG_OUT" \
  "$FAILED_TEXT_OUT" \
  "$FAILED_STATUS_OUT" \
  "$FAILED_SESSION_OUT"

set +e
timeout 600s "$ROOT/scripts/claude_e2e_compare.sh" "$ROOT" "$MODE" "$NAME" "$PROMPT_FILE"
RC=$?
set -e

python - "$RC" "$MODE" "$NAME" "$CLAUDE_MODEL" "$STREAM_OUT" "$DEBUG_OUT" "$FAILED_STREAM_OUT" "$FAILED_DEBUG_OUT" "$STATUS_JSON" <<'PY'
import json, pathlib, re, sys

rc = int(sys.argv[1])
mode = sys.argv[2]
name = sys.argv[3]
model = sys.argv[4]
stream_path = pathlib.Path(sys.argv[5])
debug_path = pathlib.Path(sys.argv[6])
failed_stream_path = pathlib.Path(sys.argv[7])
failed_debug_path = pathlib.Path(sys.argv[8])
status_path = pathlib.Path(sys.argv[9])

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

source_stream_path = stream_path if stream_path.exists() else failed_stream_path
source_debug_path = debug_path if debug_path.exists() else failed_debug_path

def sibling_with_extension(path: pathlib.Path, suffix: str) -> pathlib.Path:
    name = path.name
    if name.endswith(".stream.jsonl"):
        return path.with_name(name[:-len(".stream.jsonl")] + suffix)
    return path.with_suffix(suffix)

status_file_path = sibling_with_extension(stream_path, ".status")
failed_status_file_path = sibling_with_extension(failed_stream_path, ".status")

effective_rc = rc
for candidate in (status_file_path, failed_status_file_path):
    if candidate.exists():
        text = candidate.read_text(errors="ignore").strip()
        if text:
            try:
                effective_rc = int(text)
                break
            except ValueError:
                pass

if source_stream_path.exists():
    for line in source_stream_path.read_text(errors="ignore").splitlines():
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

if source_debug_path.exists():
    text = source_debug_path.read_text(errors="ignore")
    for match in re.finditer(r"No available accounts: ([^\n]+)", text):
        result["notes"].append(match.group(1).strip())
        break
    if "Text file busy" in text or "text file busy" in text:
        result["notes"].append("text_file_busy")

result["error_statuses"] = sorted(set(result["error_statuses"]))
result["exit_code"] = effective_rc
result["ok"] = (
    effective_rc == 0
    and result["completed"]
    and not result["result_is_error"]
    and not result["error_statuses"]
)

status_path.write_text(json.dumps(result, ensure_ascii=True))
print(json.dumps(result, ensure_ascii=True))
PY
