# Triage Labels

Canonical triage roles map directly to these labels:

| Role | Label |
|---|---|
| Needs initial triage | `needs-triage` |
| Needs information | `needs-info` |
| Fully specified for an AFK agent | `ready-for-agent` |
| Requires human work or judgment | `ready-for-human` |
| Intentionally declined | `wontfix` |

`ready-for-agent` means the issue is sufficiently specified. It does not authorize Sandcastle execution. `agent:implement` is the one-shot execution command; `agent:in-progress` and `agent:blocked` are transient machine state.
