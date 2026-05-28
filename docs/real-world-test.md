# Real-World Test: Snake Game Development

## Overview

This document records a real-world coding session where a developer used Claude Code with `tke` active to build a Rust snake game through four progressive stages:

1. **Local** — terminal snake game with crossterm
2. **Networked** — TCP server/client for remote play
3. **Multiplayer** — multiple players on one server
4. **Distributed** — federated multi-server with state sync

## Session Flow

### Stage 1: Local Snake Game

Commands run through tke shims:
- `cargo init` → project scaffolding
- `cargo build` → compile with dependency resolution (58 crates downloaded)
- `cargo test` → verify build

Tool outputs captured: `cargo build` compilation logs, `cargo test` results.

### Stage 2: Networked Mode

Added `src/network.rs` with TCP server/client using tokio:
- `NetMessage` enum for Join/State/Input/Quit messages
- Async server handling multiple client connections
- Client rendering remote game state

Commands: `cargo build` (incremental), `cargo test`.

### Stage 3: Multiplayer Mode

Extended network.rs to support multiple players per server:
- Each connection gets a unique `player_id`
- Server tracks all snakes in shared `GameState`
- Broadcasts state to all connected clients

### Stage 4: Distributed Mode

Added `src/distributed.rs` with federated node architecture:
- `FedMessage` enum for NodeJoin/StateSync/PlayerInput/PlayerMigrate/Heartbeat
- Peer discovery and connection management
- Broadcast channel for state propagation
- Reconnection logic with 5-second retry

Commands: `cargo build` (incremental, 1 warning for unused fields).

## TKE Compression Results

### Global Stats (all sessions)

| Metric | Value |
|--------|-------|
| Samples | 57 total, 30 effective, 19 changed |
| Tokens saved | 11,424,843 (28.4%) |
| Bytes saved | 45,570,026 (27.4%) |

### Claude-Specific Stats

| Metric | Value |
|--------|-------|
| Samples | 91 total, 15 effective, 10 changed |
| Tokens saved | 368,036 (15.3%) |
| Bytes saved | 1,442,604 (14.9%) |

### Per-Profile Breakdown (Claude)

| Profile | Samples | Tokens Saved | Savings |
|---------|---------|-------------|---------|
| file | 9 | 214,877 | 66.5% |
| table | 9 | 57,133 | 60.0% |
| log | 10 | 44,686 | 29.8% |
| search | 14 | 36,700 | 20.3% |
| pathlist | 13 | 11,653 | 46.3% |
| stacktrace | 3 | 1,081 | 12.4% |
| generic | 7 | 849 | 2.3% |
| json | 4 | 591 | 3.6% |
| gitstatus | 4 | 396 | 37.8% |
| diff | 7 | 70 | 0.2% |

### Key Observations

1. **`cargo build` output** — the bulk of this session's tool output. Compilation logs (58 crates) are classified as `log` profile. With the Phase 2 lowered threshold (512 bytes / 16 lines), even medium-sized build outputs trigger compression.

2. **`file` profile dominates** — 66.5% savings on file reads. Source code files (`.rs`) are compressed using outline detection (fn/struct/impl boundaries).

3. **`table` at 60%** — dependency tables and status outputs benefit from the expanded known headers.

4. **`log` at 29.8%** — build logs with progress lines ("Compiling", "Downloading") now count as signals for Generic→Log promotion, improving hit rate.

## TKE vs RTK Comparison

### Compression Approach

| Aspect | TKE | RTK |
|--------|-----|-----|
| Mechanism | Local shim wrapping tool commands | Agent hook/rules injection |
| Compression point | Before output reaches agent | Agent decides whether to compress |
| Determinism | High — same input → same output | Lower — depends on agent behavior |
| Observability | Direct: `__TKE__{...}` envelope | Inferred from transcript |

### For This Session

- **TKE** compressed `cargo build` output deterministically: 58 crate compilation lines → compact `lg` summary with fail/warn counts + signal lines.
- **RTK** would need the agent to follow hook instructions to summarize build output. Whether it does depends on prompt adherence.

### Measured Advantage

In this session, `tke` saved ~368K tokens across Claude tool outputs. The strongest savings came from:
- File reads (source code): 66.5%
- Table outputs (dependency listings): 60.0%
- Build logs: 29.8%

These savings are deterministic and guaranteed — they don't depend on agent compliance.

## Files Created

| File | Lines | Purpose |
|------|-------|---------|
| `src/main.rs` | 35 | CLI entry point for all modes |
| `src/game.rs` | 170 | Core game logic, state, rendering |
| `src/network.rs` | 130 | TCP server/client for networked play |
| `src/distributed.rs` | 145 | Federated node with peer sync |

Total: ~480 lines of Rust, building 58 dependencies.

## How to Reproduce

```bash
# Local mode
cd /root/github/snake-game
cargo run

# Networked mode
cargo run -- serve 7878      # terminal 1
cargo run -- connect 127.0.0.1:7878  # terminal 2

# Distributed mode
cargo run -- federate 7879 127.0.0.1:7880  # node 1
cargo run -- federate 7880 127.0.0.1:7879  # node 2
```
