# Installation Prerequisites

Everything needed to build and run xHoly from source.

## Required Software

### Rust toolchain (stable)

```sh
# Windows
winget install Rustlang.Rustup
rustup default stable

# macOS / Linux
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup default stable
```

Verify: `cargo --version` (1.70+; developed against 1.99)

### Scala / sbt

```sh
# JDK 21 (required by Scala 3.3 LTS)
winget install EclipseAdoptium.Temurin.21   # Windows
brew install openjdk@21                      # macOS
sudo apt install openjdk-21-jdk              # Debian/Ubuntu

# sbt
winget install sbt.sbt                       # Windows
brew install sbt                             # macOS
# Linux: download sbt launcher from https://www.scala-sbt.org/download.html
```

Verify: `java -version`, `sbt sbtVersion`

### C/C++ toolchain (for FFI/native deps)

- **Windows**: Visual Studio 2022 Build Tools ("Desktop development with C++")
- **macOS**: `xcode-select --install`
- **Linux**: `build-essential` / `gcc` / `clang`

### Git

```sh
winget install Git.Git      # Windows
git --version
```

### Vulkan-capable GPU driver

Required for the hardware backend (`wgpu`). Any recent driver works;
Vulkan 1.1+ minimum, 1.3 recommended for ray tracing / mesh shaders.

## Optional but Recommended

- **VS Code** + extensions: rust-analyzer, Metals (Scala), Even Better TOML
- **Docker** — reproducible build environment
- **cargo-deny**, **cargo-nextest**, **sccache** — dev tooling
- **CodeQL CLI** — security scanning
- **Windows App SDK** — only if packaging as MSIX
- **Physical optical drive** — for `--mount D:\` disc support

## Quick sanity build

```sh
cargo build --release        # Rust core → target/release/xholy
cd scala && sbt assembly     # JVM runtime → target/scala-*/xholy-runtime.jar
./target/release/xholy --drives    # list detected optical drives
./target/release/xholy --mount game.iso --list   # list games on a disc
```

## Platform targets

| Target | Status |
|---|---|
| `x86_64-pc-windows-msvc` | Primary dev target |
| `x86_64-unknown-linux-gnu` | Supported |
| `aarch64-apple-darwin` | Supported |
| `armv7-unknown-linux-gnueabihf` | Raspberry Pi 4 (software backend) |

Cross-compile examples:

```sh
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```
