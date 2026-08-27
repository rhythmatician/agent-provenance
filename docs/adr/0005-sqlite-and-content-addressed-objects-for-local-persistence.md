---
status: accepted
---

# Use SQLite and content-addressed objects for local persistence

SQLite will own ordered event metadata, transactional appends, schema versions, and rebuildable projections. Large retained payloads will live outside the database in a content-addressed object directory referenced by digest, keeping the event database inspectable and bounded while preserving local-first crash recovery.
