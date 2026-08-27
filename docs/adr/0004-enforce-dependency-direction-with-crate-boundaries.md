---
status: accepted
---

# Enforce dependency direction with crate boundaries

The workspace dependency direction is `domain ← core ← adapters ← CLI`, with acceptance tests allowed to depend on every layer. Cargo crate boundaries make reverse dependencies unrepresentable without an explicit manifest change, and a repository guardrail rejects forbidden internal dependency edges before compilation.
