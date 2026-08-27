# Documentation Policy

Repository prose must not become a second source of truth for facts the repository can express executably. Code explains mechanics. Documentation explains meaning.

## Executable artifacts own current reality

Current behavior, structure, interfaces, contracts, configuration, supported states, and implementation status belong in code, types, tests, schemas, configuration, and generated artifacts derived from them.

Do not commit standalone implementation summaries, architecture summaries, project outlines, TODO lists, status reports, deliverables lists, or manually synchronized implementation inventories.

## `CONTEXT.md` owns domain language only

`CONTEXT.md` defines project-specific concepts and canonical terms. It must not contain implementation details, plans, TODOs, status, file inventories, or algorithms.

## ADRs own architectural rationale

Create an ADR only when the decision is hard to reverse, surprising without context, and the result of a genuine trade-off. ADRs are historical records. Mark a replaced ADR as superseded and point to its successor instead of rewriting history.

## GitHub owns work state

Requirements, plans, acceptance criteria, dependencies, current work, investigations, and completion state belong in GitHub Issues and pull requests. Temporary pre-remote drafts may live under gitignored `.scratch/` only until they are published or discarded.

## Navigation stays thin

README and index files may state what the project is, how to run its canonical verification command, and where authoritative material lives. They should point instead of reproduce.

## External research is version-bound

Commit expensive-to-reconstruct external research only when it names the exact upstream revision or artifact digest, identifies evidence locations, and makes no claim to describe a later revision.

## One fact, one home

Before committing prose, ask why the information cannot live in code, tests, configuration, a contract, GitHub, or an existing canonical document. If it already has an authority, link to that authority instead of copying it.
