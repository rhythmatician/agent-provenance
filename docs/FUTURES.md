# Preserved Futures

These entries preserve concrete options. They are not commitments, milestones, or current work.

## Semantic provenance graph and query layer

Trigger: the raw event recorder, deterministic projections, loss semantics, and workspace-state model have demonstrated trustworthy behavior on real sessions.

Research Graphify's current GitHub implementation and the retained Graphify skill references before fixing graph storage, extraction, or query interfaces. Reuse useful patterns where they fit, but preserve Agent Provenance's explicit distinction among observations, deterministic derivations, inferences, and claims.

## Factory analytics

Trigger: enough comparable sessions exist to support stable aggregate measurements.

Possible measurements include time to first reproduction, validation latency, stale validation at completion, repeated failed commands, abandoned hypotheses, feedback-loop cost, and recurring human interventions.

## Active agent evidence feedback

Trigger: single-session queries are reliable enough that an agent can consume them without being misled by incomplete capture.

A running agent could query current-state validation, failed approaches, unresolved failures, and evidence supporting a completion claim.

## Multi-machine sessions

Trigger: a real workflow spans hosts or containers and cannot be represented faithfully as one local recorder session.

Any distributed design must preserve source-local ordering, clock uncertainty, and observation gaps rather than inventing a global total order.
