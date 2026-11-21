# AI Agents and Co-Authorship

This document records AI agents that contribute to xHoly and sets the commit-attribution convention. Update the table when a new agent makes a substantive contribution.

## Agent Roles

| Agent | Role | Contributions |
|---|---|---|
| Devin AI Integration (`devin-ai-integration[bot]`, `158243242+devin-ai-integration[bot]@users.noreply.github.com`) | AI software engineering agent | May implement assigned changes and contribute research, tests, or reviews. |
| GitHub Copilot | AI coding assistant | May implement changes, debug issues, and contribute documentation or tests. |

These are role descriptions, not a claim that every agent contributed to every change. Record only agents that actually contributed to a given commit.

## Commit Attribution

When an AI agent contributes to a commit, include that agent as a co-author using a `Co-authored-by:` trailer. Use the agent's recognized name and verified email; do not guess an identity. Add new agents to the table when they contribute.

Example with GitHub Copilot as a co-author:

```sh
git commit -m "Describe change" \
  -m "Co-authored-by: GitHub Copilot <copilot@users.noreply.github.com>"
```

For multiple contributors, add one `-m "Co-authored-by: Name <email>"` argument for each co-author. `git commit --author="Name <email>"` sets the commit's primary author; it does not add a co-author. Use `--author` when the agent is the primary author, and use the trailer for each distinct co-author.

## Why Track Co-Authorship

Co-authorship makes AI contributions transparent, gives contributors appropriate credit, and leaves a reproducible record of which agents participated in a change.
