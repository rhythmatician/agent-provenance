# Agent Provenance

Agent Provenance records what an automated engineering session observed and preserves enough provenance to distinguish evidence from interpretation.

## Evidence

**Observation**:
A record emitted by a capture source about something it directly detected during a session.
_Avoid_: Fact, conclusion, interpretation

**Claim**:
A statement made by an agent, user, or tool that may or may not be supported by observations.
_Avoid_: Finding, fact

**Deterministic Derivation**:
A conclusion reproducibly computed from observations by a versioned rule.
_Avoid_: Inference, observation

**Inference**:
A revisable interpretation whose correctness is not guaranteed solely by its source observations.
_Avoid_: Fact, deterministic derivation

**Observation Gap**:
An explicit record that a capture source could not observe some interval, scope, or event class completely.
_Avoid_: Missing log, warning

**Provenance Link**:
A relationship from a derivation, inference, or claim assessment back to the observations and rule that produced it.
_Avoid_: Reference, citation

## Execution

**Session**:
One bounded attempt to execute a root command and record the resulting evidence.
_Avoid_: Run, trace, job

**Capture Source**:
A producer of observations with a declared scope and coverage.
_Avoid_: Observer, sensor, collector

**Process Instance**:
One execution lifetime identified independently of an operating-system process identifier, which may be reused.
_Avoid_: PID, process

**Event Stream**:
The ordered sequence in which a recorder durably accepts observations for one session. Stream order does not by itself prove causal order.
_Avoid_: Log file, transcript

## Workspace and validation

**Workspace State**:
The recorder's identified state of the relevant workspace at a point in a session.
_Avoid_: Commit, snapshot

**Validation Evidence**:
A successful or failed check associated with the workspace state against which it executed.
_Avoid_: Green check, proof

**Evidence Freshness**:
Whether validation evidence applies to the current workspace state, is stale because the state changed, or is indeterminate because continuity is incomplete.
_Avoid_: Validity, confidence

**Projection**:
A disposable, rebuildable view deterministically produced from an event stream.
_Avoid_: Source of truth, cache
