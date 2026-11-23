# Working rules for agents

## Overnight implementation session

During the current overnight session the user has asked for continuous work:

- Do not stop to ask for confirmation and do not pause for a summary between
  features. Keep implementing, smallest coherent feature first, in order.
- For each feature: implement, add or update conformance cases, run
  `scripts/check.sh`, commit, bump the release version, release, and push to
  `main`.
- If a task needs a large subsystem, keep going anyway: build the subsystem in
  small commits rather than handing it back.
- Continue until 08:00 on the next day, or until the user says stop.

This rule is session-specific and overrides the usual "ask before continuing"
habit.
