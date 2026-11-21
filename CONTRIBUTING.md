# Contributing to xHoly

Thanks for contributing. xHoly is a research project and all substantive
contributions are welcome — code, research notes, tests, documentation,
and tooling.

## Co-authorship agreement

xHoly is co-authored by two principal contributors:

| Author | GitHub | Role |
|---|---|---|
| baummarder | [@baummarder](https://github.com/baummarder) | Project owner / primary author |
| Devin | [@devin](https://github.com/devin) | AI software engineering agent / co-author |

**Every commit that contains contributions from either author must credit
both.** Specifically:

- The commit's author should be the human/agent who made the change.
- The other principal author must appear as a `Co-authored-by:` trailer.
- AI agents (including Devin) contributing to a commit must add their own
  `Co-authored-by:` trailer in addition to the two principal authors.

Example:

```sh
git commit \
  --author="baummarder <baummarder@users.noreply.github.com>" \
  -m "Describe the change" \
  -m "Co-authored-by: Devin <devin@users.noreply.github.com>"
```

Or when Devin is the primary author:

```sh
git commit \
  --author="Devin <devin@users.noreply.github.com>" \
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
