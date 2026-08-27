# Engineering Workflow

Wayfinder owns uncertain, human-in-the-loop planning. Research, prototypes, and design choices remain there until the desired outcome and expensive-to-reverse decisions are settled.

Sandcastle executes only bounded implementation tickets that satisfy `docs/agents/tracer-contract.md`. It does not invoke Wayfinder or invent unresolved product or architectural decisions.

Methodology belongs in installed skills:

- `/implement`: execute a fully specified change.
- `/tdd`: build new behavior through RED → minimal GREEN → refactor vertical slices.
- `/diagnosing-bugs`: reproduce and rank hypotheses before repairing a reported failure.
- `/code-review`: perform independent specification and standards review in fresh context.

Factory code owns eligibility, claims, permissions, branches/worktrees, retries, validation, and merge state. Prompts carry only ephemeral role context. Repeated review findings should migrate into the lowest reliable machine-enforced layer.
