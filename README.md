# Agent Provenance

Agent Provenance is a local-first execution recorder for evidence produced during automated software-engineering sessions. It keeps observations authoritative, preserves uncertainty explicitly, and treats every higher-level interpretation as rebuildable.

The repository name is a working title. The installed binary is `provenance`.

## Repository map

- `crates/provenance-domain`: dependency-free domain language and raw-event types.
- `crates/provenance-core`: recording workflow and the public store, clock, ID, and capture seams.
- `crates/provenance-adapters`: side-effecting storage, clock, and platform-capture adapters.
- `crates/provenance-cli`: composition root and user-facing binary.
- `tests/provenance-acceptance`: behavior tests crossing only public crate seams.

## Prerequisites

- stable Rust with `rustfmt` and Clippy; the workspace minimum is Rust 1.85;
- Python 3.11 or later for the dependency-direction guardrail.

## Start

```bash
cargo run -p provenance-cli -- --help
bash scripts/verify.sh
```

On PowerShell:

```powershell
cargo run -p provenance-cli -- --help
./scripts/verify.ps1
```

Canonical domain language is in [`CONTEXT.md`](CONTEXT.md). Architectural rationale is in [`docs/adr/`](docs/adr/). The documentation authority index is [`docs/INDEX.md`](docs/INDEX.md).
