# Contributing to Clockwork

Keep changes focused and include a short explanation of the problem and resulting behavior.
Read [README.md](README.md) for the project layout and import boundaries, and
[DESIGN.md](DESIGN.md) before changing the editor's interaction or visual design.

## Development

Install Rust with rustup, plus your platform's native build prerequisites. The repository pins
Rust 1.98.1 with `rustfmt` and `clippy` in `rust-toolchain.toml`. Windows development uses the
MSVC toolchain, C++ build tools and Windows SDK.
Use the repository's `Cargo.lock`; dependency updates should be deliberate.

Before submitting a change, run from the repository root:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

For editor changes, also launch `cargo run --locked -p clockwork_editor` and exercise the
changed interaction. Use `CLOCKWORK_DATA_DIR` with an isolated writable folder when testing
Library imports or settings, so your personal Library is not changed. Add focused regression
tests for parsing, persistence and behavior changes; do not require copyrighted game files
or external network services in the test suite.

## Bug reports and pull requests

Use this repository's Issues or Pull requests tab. For a bug, include the Clockwork version
or commit, operating system, reproduction steps, expected result and actual result. For
graphics or controller bugs, include the GPU/driver or controller model and connection type.
Use a small synthetic fixture when a file is needed to reproduce a parser issue.

Do not attach ROMs, disc images, BIOS files, private credentials or extracted copyrighted
content. Redact personal paths and tokens from logs. For import issues, the platform, file
format and a description of the failing validation usually provide a useful starting point.

Contributions are made under the repository's MIT OR Apache-2.0 licensing terms. Preserve
third-party attribution when adding dependencies or licensed assets. See
[docs/RELEASING.md](docs/RELEASING.md) for the release process.
