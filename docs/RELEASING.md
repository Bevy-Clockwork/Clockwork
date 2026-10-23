# Preparing a GitHub release

The initial release is **v0.1.0**, marked as a **prerelease**, with a Windows x64 ZIP. Preparing
files locally does not publish a release. The tag workflow creates a draft for review; a
maintainer publishes it after inspecting the artifacts and release notes.

The release repository is [Bevy-Clockwork/Clockwork](https://github.com/Bevy-Clockwork/Clockwork).
Confirm that `origin` points to this repository before pushing release commits or tags.

## Before building

- Confirm the intended GitHub repository and push destination. Do not substitute a guessed
  owner or repository name in release links.
- Work from the commit intended for release with no uncommitted changes. Inspect `git status`
  and `git diff`, including untracked files, before committing the release preparation.
- Confirm `[workspace.package].version` in `Cargo.toml` matches the release tag without its
  `v` prefix. Keep `Cargo.lock` up to date and commit it.
- Review `CHANGELOG.md`, README import limits and controller limitations. Do not describe
  identification or extraction support as game emulation.
- Confirm the package contains no ROMs, disc images, BIOS files, private content folders,
  credentials or local user settings. Review any newly included assets and dependencies.

## Validate

Use Windows x64 with the pinned Rust 1.98.1 MSVC toolchain, the Visual Studio C++ build tools,
Windows SDK and PowerShell 7 (`pwsh`). Install the pinned dependency-license report tool:

```powershell
cargo install --locked --version 0.9.2 --features cli cargo-about
```

Run from the repository root:

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
pwsh ./scripts/package-release.ps1 -Tag v0.1.0
```

The packaging script checks the tag against the workspace version, builds the editor and
MCP server in release mode using the lockfile, and runs `scripts/generate-notices.ps1`.
It produces:

```text
dist/clockwork-0.1.0-windows-x86_64.zip
dist/SHA256SUMS.txt
```

The ZIP must contain `clockwork.exe`, `clockwork-mcp.exe`, `.mcp.json`, `README.md`,
`CHANGELOG.md`, `DESIGN.md`, `CONTRIBUTING.md`, `docs/RELEASING.md`, `LICENSE-MIT`,
`LICENSE-APACHE` and `THIRD-PARTY-NOTICES.html`. The packaged
`.mcp.json` must start `./clockwork-mcp.exe`, rather than use the source checkout's Cargo
command. The dependency notice file is
generated for the release; retain it when redistributing the binaries. To package release
binaries that have already been built and validated, use
`pwsh ./scripts/package-release.ps1 -SkipBuild`.

Verify the ZIP hash against `SHA256SUMS.txt`:

```powershell
Get-FileHash ./dist/clockwork-0.1.0-windows-x86_64.zip -Algorithm SHA256
Get-Content ./dist/SHA256SUMS.txt
```

Extract the ZIP into a fresh folder and run the packaged applications, rather than the
development executables. Use a fresh `CLOCKWORK_DATA_DIR` for the smoke test. Check:

- The editor opens to a usable Library without requiring source files or a Rust toolchain.
- Import validation handles a synthetic supported fixture and an unsupported file.
- Settings → Controls opens, keyboard rebinding persists after restart, and reset works.
- Any available controller is detected, its prompts match the selected style, and gameplay
  inputs work. Record which hardware was tested; do not imply untested devices were verified.
- The packaged `clockwork-mcp.exe` starts and completes an MCP connection with a client.
- No runtime DLL or asset dependency is missing from the packaged application.

The Windows executables link to `VCRUNTIME140.dll`. The installation instructions must retain
the Microsoft Visual C++ v14 x64 Redistributable requirement and official download link.
Check a machine without a development toolchain when validating the end-user install path;
developer machines may already have this runtime installed.

Record actual validation results and any untested cases in the release description. An
unsigned build should be identified as such. Back up personal data before testing migrations
against an existing Library.

## Create and review the GitHub draft

The [release workflow](../.github/workflows/release.yml) supports a manual run for building
downloadable workflow artifacts without creating a GitHub release. Use this for a candidate
build before tagging. The [CI workflow](../.github/workflows/ci.yml) checks source changes.

After the release commit has been reviewed and pushed to the intended repository, a
maintainer can create and push the matching tag:

```powershell
git tag -a v0.1.0 -m "Clockwork 0.1.0 preview"
git push origin v0.1.0
```

Tagging starts the release workflow, using `CHANGELOG.md` for the initial draft notes.
Wait for validation and packaging to complete, then
review the resulting **draft prerelease** in GitHub's Releases tab. Confirm that the draft
points to the intended commit, includes the ZIP and checksum file, and accurately describes
the scope and limitations from `CHANGELOG.md`. Download and verify the uploaded ZIP before
publishing the draft.

For future versions, update the workspace version, lockfile, changelog and versioned examples
together. Do not replace an already published artifact under the same version; issue a new
version for corrections.
