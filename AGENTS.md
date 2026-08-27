# AGENTS.md

## Coding

Use YAGNI and DRY. Build new behavior through strict RED → minimal GREEN → refactor vertical slices. Prefer deep modules with small interfaces and test through the highest public seam that exposes the behavior.

The domain and core crates forbid unsafe code. Any future unsafe code must remain inside a platform adapter, carry a local `SAFETY:` justification, and be exercised through the adapter's public seam.

## Verification

Run the repository verification entry point before claiming completion:

- Unix/WSL: `bash scripts/verify.sh`
- PowerShell: `./scripts/verify.ps1`

## Agent skills

### Issue tracker

Work belongs in GitHub Issues for the repository selected by `origin`. Pull requests are not a request surface. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical triage roles map 1:1 to `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, and `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repository: `CONTEXT.md` contains domain language and `docs/adr/` contains architectural rationale. See `docs/agents/domain.md`.

### Workflow

Wayfinder owns uncertain human-in-the-loop planning. Sandcastle may execute only fully specified tracer-bullet implementation tickets. Use `/implement`, `/tdd`, `/diagnosing-bugs`, and `/code-review` for their respective methodology. See `docs/agents/workflow.md` and `docs/agents/tracer-contract.md`.

### Documentation

Code, tests, contracts, and configuration own current mechanics. GitHub owns requirements and work state. `CONTEXT.md` owns domain language. ADRs own architectural rationale. Do not commit implementation summaries, plans, TODO/status documents, or prose that duplicates executable truth. See `docs/agents/documentation.md` and `docs/INDEX.md`.
