# Real E2E Matrix

This file is generated from the current local E2E artifacts.

## Stable Cases

### Codex

| Case | Raw | TKE | RTK Rules | Notes |
| --- | --- | --- | --- | --- |
| `buildcase` | pass | pass | missing | stable tke case |
| `fairbuild` | ungraded | missing | missing | - |
| `fairfind` | ungraded | missing | ungraded | fair RTK sample |
| `fairrg` | ungraded | missing | ungraded | fair RTK sample |
| `findcase` | pass | pass | missing | stable tke case |
| `realtask` | pass | pass | missing | stable tke case |
| `rgcase` | pass | pass | missing | stable tke case |

### Claude

| Case | Raw | TKE | RTK Hook | Notes |
| --- | --- | --- | --- | --- |
| `findcase` | gateway_error | fail | gateway_error | experimental live tke path |

## Fairness Rules

- Codex vs RTK must use `rtk-codex-rules`.
- Claude vs RTK must use `rtk-hook`.
- `rtk-direct` is not the official fairness path for Codex.

## Current Repo Verdict

- Codex remains the primary validated live-compression path.
- Claude currently prioritizes stable compatibility over live compression by default.
- RTK results must be reported per agent integration mode, not as one universal number.
