# AI Agent Co-Authors

This file records the AI agents that contribute to xHoly. Update the table when another agent contributes.

| Agent | Role | Contributions |
|---|---|---|
| Devin | AI software engineering agent | Implements assigned changes and may contribute research, tests, and reviews. |
| GitHub Copilot | AI coding assistant | Assists with implementation, debugging, documentation, and tests. |

## Commit Attribution

Whenever GitHub Copilot or another agent contributes to a commit that it pushes, identify that agent in the commit's `Co-authored-by` trailer. Use the agent's recognized name and email from this table (or add the agent to the table first).

For example, to record GitHub Copilot as a co-author while preserving the primary author's identity:

`git commit --author="Primary Author <primary-author@example.com>" -m "Describe change" -m "Co-authored-by: GitHub Copilot <copilot@users.noreply.github.com>"`

For another agent, replace the trailer's name and email with that agent's recognized identity. `--author` sets the commit's primary author; it does not add a co-author. If an agent is the primary author, use its identity with `--author`; add a `Co-authored-by:` trailer for each distinct agent co-author.

## Why Track Co-Authorship

Co-authorship makes AI contributions transparent, ensures contributors receive appropriate credit, and provides a reproducible record of which agents participated in the project's changes.
