# Real E2E Matrix

This file tracks the current real end-to-end cases and the repo's current confidence level in each path.

## Stable Cases

### Codex

| Case | Raw | TKE | Notes |
| --- | --- | --- | --- |
| `findcase` | pass | pass | stable path-list compression case |
| `buildcase` | pass | pass | stable build-log compression case |
| `rgcase` | pass | pass | strong code-reading/search compression case |
| `realtask` | pass | pass | correct but currently too small to compress |

### Claude

| Case | Raw | TKE | RTK Hook | Notes |
| --- | --- | --- | --- | --- |
| `findcase` | pass | fail | pass | current Claude fairness baseline |

## Fairness Rules

- Codex vs RTK must use `rtk-codex-rules`
- Claude vs RTK must use `rtk-hook`
- `rtk-direct` is not the official fairness path for Codex

## Current Repo Verdict

- Codex is the primary validated path for `tke`
- Claude still needs a compatibility-safe compression path before `tke` can be considered stable there
- RTK fairness results must be reported per integration mode, not as a single universal number
