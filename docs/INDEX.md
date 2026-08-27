# Documentation Authority and Traceability Index

## Authority order

1. Code, tests, contracts, and configuration: current behavior and mechanics.
2. GitHub Issues and pull requests: requirements, work state, and implementation history.
3. `CONTEXT.md`: canonical domain language.
4. Accepted ADRs under `docs/adr/`: architectural rationale.
5. Version-pinned external references: expensive-to-reconstruct grounding.
6. Git history: historical evidence.

## Architecture

- `docs/adr/0001-authoritative-observations-and-rebuildable-interpretations.md`
- `docs/adr/0002-platform-specific-capture-behind-a-portable-core.md`
- `docs/adr/0003-workspace-state-scopes-validation-evidence.md`
- `docs/adr/0004-enforce-dependency-direction-with-crate-boundaries.md`
- `docs/adr/0005-sqlite-and-content-addressed-objects-for-local-persistence.md`

## Future optionality

- `docs/FUTURES.md`: concrete options preserved for later consideration; not current work.

## Control plane

- `AGENTS.md` and `docs/agents/*`: agent and factory behavior.
- `.github/CODEOWNERS`: protected control-plane ownership.
