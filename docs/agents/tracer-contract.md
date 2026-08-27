# Tracer-Bullet Ticket Contract

Every `agent:implement` issue must describe a bounded vertical slice that one fresh agent session can complete without inventing scope. `ready-for-agent` indicates readiness; `agent:implement` authorizes execution.

An AFK-ready issue must contain all seven concepts:

1. A bounded observable outcome.
2. No unresolved design or product decision.
3. Explicit, checkable acceptance criteria.
4. An explicit verification path.
5. Dependencies and blockers, including `none` when there are none.
6. Scope small enough for one implementation session.
7. A vertical or tracer-bullet shape rather than a horizontal layer batch.

Reject issues whose acceptance criterion is only “works correctly,” that leave `TBD` decisions, omit blockers, or ask an agent to build the entire system in one pass.
