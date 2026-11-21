# Contributing to xHoly

Thanks for contributing. xHoly is a research project and all substantive
contributions are welcome — code, research notes, tests, documentation,
and tooling.

## Co-authorship agreement

xHoly credits the project owner and AI agents that made substantive contributions:

| Author | GitHub | Role |
|---|---|---|
| baummarder | [@baummarder](https://github.com/baummarder) | Project owner / primary author |
| Devin AI Integration | `devin-ai-integration[bot]` | AI software engineering agent / co-author |

Credit only contributors who made a substantive change. Specifically:

- The commit's author should be the human/agent who made the change.
- Add a `Co-authored-by:` trailer for each distinct contributor who also made a substantive contribution.
- AI agents must use their verified bot identity, not a human account with a similar name.

Example:

```sh
git commit \
  --author="baummarder <baummarder@users.noreply.github.com>" \
  -m "Describe the change" \
  -m "Co-authored-by: Devin AI Integration <158243242+devin-ai-integration[bot]@users.noreply.github.com>"
```

Or when Devin AI Integration is the primary author:

```sh
git commit \
  --author="Devin AI Integration <158243242+devin-ai-integration[bot]@users.noreply.github.com>" \
  -m "Describe the change" \
  -m "Co-authored-by: baummarder <baummarder@users.noreply.github.com>"
```

See also `Agents.md` for the full agent roster and attribution rules.

## Development setup

- **Rust**: stable toolchain (`cargo build --release`)
- **Scala**: JDK 17+, sbt 1.10+ (`cd scala && sbt compile`)
- Targets: `x86_64`, `aarch64`, `armv7` (Raspberry Pi 4)

## Style

- Rust: `cargo fmt`, `cargo clippy -- -D warnings`
- Scala: scalafmt defaults, 2-space indent
- Keep commits focused; explain *why* in the message body

## License

By contributing you agree your contributions are licensed under the
Apache License 2.0 (see `LICENSE`).
