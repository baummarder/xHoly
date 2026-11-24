# Third-party licenses

xHoly's own code is licensed under the Apache License 2.0 (see [LICENSE](LICENSE)).
The components below are third-party. Their licenses apply to them, not to
xHoly's license. Apache-2.0 does not grant rights to Microsoft SDKs, Xbox
software, games, firmware, trademarks or other third-party code.

## Direct Rust dependencies

Generated from `cargo metadata` on 2026-11-24 (direct dependencies of the
`xholy` crate, including build and dev dependencies). Transitive
dependencies are not listed here; the complete set can be produced with
`cargo metadata --format-version 1`.

| Crate | Version | License (from crate metadata) |
|---|---|---|
| ab_glyph | 0.2.32 | Apache-2.0 |
| cc | 1.11.3 | MIT OR Apache-2.0 |
| chrono | 0.4.45 | MIT OR Apache-2.0 |
| clap | 4.6.9 | MIT OR Apache-2.0 |
| criterion | 0.5.1 | Apache-2.0 OR MIT |
| crossterm | 0.28.1 | MIT |
| discord-rich-presence | 1.1.0 | MIT |
| env_logger | 0.11.12 | MIT OR Apache-2.0 |
| image | 0.25.10 | MIT OR Apache-2.0 |
| log | 0.4.35 | MIT OR Apache-2.0 |
| notify | 8.2.2 | CC0-1.0 |
| pollster | 0.4.0 | Apache-2.0/MIT |
| ratatui | 0.29.0 | MIT |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 |
| thiserror | 2.0.21 | MIT OR Apache-2.0 |
| toml | 0.8.23 | MIT OR Apache-2.0 |
| wgpu | 24.0.5 | MIT OR Apache-2.0 |
| winit | 0.30.13 | Apache-2.0 |

Full license texts are distributed with each crate. Crate metadata is the
source of the license column; it has not been separately legal-reviewed.

## Bundled assets

| Asset | License | Location |
|---|---|---|
| Inter (font) | SIL Open Font License 1.1 | `assets/fonts/Inter.ttf`, `assets/fonts/OFL-Inter.txt` |

## Not included

- No Microsoft Xbox GDK, GDKX or other Microsoft SDK code is included. The Rust crate has no Microsoft SDK dependency; a full SDK-independence audit (including build tooling) has not been completed.
- No Xbox firmware, Xbox games or game dumps are included.
- No code is copied from ScarlettX; it is used as a compatibility reference only.