#!/usr/bin/env bash
set -euo pipefail
exec </dev/null

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd -- "$SCRIPT_DIR/.." && pwd)}"
HOST_TKE_BIN="${TKE_BIN:-$ROOT/target/release/tke}"
HOST_RUST_TOOLCHAIN_BIN="${HOST_RUST_TOOLCHAIN_BIN:-$(dirname "$(rustup which cargo 2>/dev/null || command -v cargo)")}"
HOST_TOOL_PATH="${HOST_TOOL_PATH:-$HOST_RUST_TOOLCHAIN_BIN:/root/.cargo/bin:/usr/local/bin:/usr/bin:/bin}"
RUN_ROOT="${RUN_ROOT:-/tmp/tke-claude-harness-check}"

if ! TKE_BIN_REAL="$(readlink -f "$HOST_TKE_BIN" 2>/dev/null)"; then
  TKE_BIN_REAL="$HOST_TKE_BIN"
fi
if [[ ! -f "$TKE_BIN_REAL" ]]; then
  echo "tke binary not found at $HOST_TKE_BIN; run cargo build --release or set TKE_BIN" >&2
  exit 2
fi
chmod +x "$TKE_BIN_REAL" 2>/dev/null || :

rm -rf "$RUN_ROOT"
mkdir -p "$RUN_ROOT/repo" "$RUN_ROOT/home/.claude" "$RUN_ROOT/bin" "$RUN_ROOT/shims"

cp "$TKE_BIN_REAL" "$RUN_ROOT/bin/tke"
chmod +x "$RUN_ROOT/bin/tke"
cp -a "$ROOT/src/." "$RUN_ROOT/repo/src/"
for f in Cargo.toml Cargo.lock README.md .gitignore; do
  cp "$ROOT/$f" "$RUN_ROOT/repo/$f"
done

ACTIVATE_SCRIPT="$(
  PATH="$RUN_ROOT/bin:$HOST_TOOL_PATH" "$RUN_ROOT/bin/tke" activate --shim-dir "$RUN_ROOT/shims" claude
)"

cat >"$RUN_ROOT/home/.bashrc" <<EOF
export PATH="$RUN_ROOT/bin:$HOST_TOOL_PATH"
export CARGO_HOME="/root/.cargo"
export RUSTUP_HOME="/root/.rustup"
export TKE_CLAUDE_LIVE_TOOLS=1
export CLAUDE_CODE_SIMPLE=1
$ACTIVATE_SCRIPT
EOF

cd "$RUN_ROOT/repo"
export HOME="$RUN_ROOT/home"
export BASH_ENV="$RUN_ROOT/home/.bashrc"
export PATH="$RUN_ROOT/bin:$HOST_TOOL_PATH"
export CARGO_HOME="/root/.cargo"
export RUSTUP_HOME="/root/.rustup"

CHECK_OUTPUT="$(bash -lc 'printf "cargo=%s\n" "$(command -v cargo)"; printf "real=%s\n" "$TKE_REAL_PATH"; cargo --version')"
printf '%s\n' "$CHECK_OUTPUT"

if ! grep -q "^cargo=$RUN_ROOT/shims/cargo$" <<<"$CHECK_OUTPUT"; then
  echo "expected shim cargo path in isolated bash env" >&2
  exit 1
fi

if ! grep -q "^real=$RUN_ROOT/bin:$HOST_TOOL_PATH$" <<<"$CHECK_OUTPUT"; then
  echo "expected isolated TKE_REAL_PATH in bash env" >&2
  exit 1
fi

timeout 120s bash -lc 'cargo test --lib -- --nocapture | tail -n 20' >/dev/null
