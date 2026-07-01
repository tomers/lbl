# Agent instructions

Guidance for AI assistants working in this repository.

## Comments

Do not add temporary or conversational comments — notes that only explain a
recent change, a rejected alternative, or how something used to work.

Add comments only when they help long-term maintainability:

- Non-obvious behavior, invariants, or constraints future readers must know
- External requirements (tooling, protocols, browser/platform limits)
- Workarounds where the reason is not clear from the code alone

Prefer self-explanatory names and structure over comments. If a comment would
become stale after the next refactor, it probably should not exist.
