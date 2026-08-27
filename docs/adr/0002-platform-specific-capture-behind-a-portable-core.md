---
status: accepted
---

# Put platform-specific capture behind a portable core

Process and filesystem tracing use materially different operating-system mechanisms, so capture adapters may expose different implementation strategies while producing the same domain observations. The domain, recording workflow, event store interface, and query semantics remain portable; the first production capture adapter targets Linux, including WSL, before a Windows adapter is implemented.
