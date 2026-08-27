# Issue Tracker

Issues live in GitHub Issues for the repository configured as `origin`.

- Use `gh repo view --json nameWithOwner -q .nameWithOwner` to resolve the repository instead of hard-coding a slug.
- GitHub Issues are authoritative for requirements, dependencies, acceptance criteria, and work state.
- Pull requests are implementation and review surfaces, not an incoming request queue.
- Planning artifacts created before the remote exists belong under the gitignored `.scratch/` directory and must be moved into GitHub or deleted once the remote is configured.
