# Real E2E Matrix

This file is generated from the current local E2E artifacts.

## Stable Cases

### Codex

| Case | Raw | TKE | RTK Rules | Notes |
| --- | --- | --- | --- | --- |
| `buildcase` | pass | pass | missing | stable tke case |
| `fairbuild` | pass | missing | missing | - |
| `fairfind` | fail | missing | fail | - |
| `fairrg` | fail | missing | fail | - |
| `findcase` | pass | pass | missing | stable tke case |
| `realtask` | pass | pass | missing | stable tke case |
| `rgcase` | pass | pass | missing | stable tke case |

### Claude

| Case | Raw | TKE | RTK Hook | Notes |
| --- | --- | --- | --- | --- |
| `fairbuild` | fail | fail | missing | experimental live tke path |
| `fairfind` | fail | fail | missing | experimental live tke path |
| `fairrg` | pass | pass | missing | live tke correct |
| `findcase` | gateway_error | fail | gateway_error | experimental live tke path, gateway noise on RTK hook path |

## Fairness Rules

- Codex vs RTK must use `rtk-codex-rules`.
- Claude vs RTK must use `rtk-hook`.
- `rtk-direct` is not the official fairness path for Codex.

## Current Repo Verdict

- Codex remains the primary validated live-compression path.
- Claude tool compression works via PATH shims; agent output is passed through.
- RTK results must be reported per agent integration mode, not as one universal number.
