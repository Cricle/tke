#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-/root/github/tke}"

rm -rf \
  "$ROOT/.tmp-claude-e2e" \
  "$ROOT/.tmp-codex-e2e" \
  "$ROOT/.tmp-codex-e2e-real" \
  "$ROOT/.tmp-codex-e2e-fair" \
  /tmp/tke-claude-e2e \
  /tmp/tke-codex-real-prompts \
  /tmp/tke-codex-fair-prompts \
  /tmp/tke-codex-fair-repo \
  /tmp/tke-rtk-codex-proj \
  /tmp/tke-e2e-shims \
  /tmp/tke-debug-shims
