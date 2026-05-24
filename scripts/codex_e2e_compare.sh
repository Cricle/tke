#!/usr/bin/env bash
set -euo pipefail
exec </dev/null

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd -- "$SCRIPT_DIR/.." && pwd)}"
TKE_BIN="${TKE_BIN:-$ROOT/target/release/tke}"
RTK_BIN="${RTK_BIN:-/tmp/rtk-bin/rtk}"
SHIM_DIR="${SHIM_DIR:-/tmp/tke-e2e-shims}"
OUT_DIR="${OUT_DIR:-$ROOT/.tmp-codex-e2e}"

mkdir -p "$OUT_DIR"

run_case() {
  local mode="$1"
  local name="$2"
  local prompt_file="$3"
  local env_prefix=()
  if [[ "$mode" == "wrapped" || "$mode" == "tke" ]]; then
    mkdir -p "$SHIM_DIR"
    eval "$("$TKE_BIN" activate --shim-dir "$SHIM_DIR" codex)"
    env_prefix=(
      env
      "TKE_BIN=$TKE_BIN"
      "TKE_SHIM_DIR=$TKE_SHIM_DIR"
      "TKE_REAL_PATH=$TKE_REAL_PATH"
      "TKE_AGENT_CMDS=$TKE_AGENT_CMDS"
      "TKE_TOOL_CMDS=$TKE_TOOL_CMDS"
      "PATH=$PATH"
    )
  fi
  if [[ "$mode" == "rtk-direct" ]]; then
    local rtk_dir
    rtk_dir="$(dirname "$RTK_BIN")"
    env_prefix=(
      env
      "PATH=$rtk_dir:$PATH"
    )
  fi

  local before
  before="$(find "$HOME/.codex/sessions" -type f -name '*.jsonl' 2>/dev/null | sort | tail -n 1 || true)"
  local json_out="$OUT_DIR/${name}.${mode}.jsonl"
  local msg_out="$OUT_DIR/${name}.${mode}.txt"

  "${env_prefix[@]}" codex exec \
    --json \
    --dangerously-bypass-approvals-and-sandbox \
    -C "$ROOT" \
    -o "$msg_out" \
    "$(cat "$prompt_file")" >"$json_out"

  local after
  after="$(find "$HOME/.codex/sessions" -type f -name '*.jsonl' 2>/dev/null | sort | tail -n 1 || true)"
  local rollout="$after"
  if [[ -n "$before" && "$before" == "$after" ]]; then
    rollout=""
  fi

  printf '%s\n' "$rollout" >"$OUT_DIR/${name}.${mode}.rollout"
}

if [[ $# -ge 3 ]]; then
  run_case "$2" "$3" "$4"
fi
