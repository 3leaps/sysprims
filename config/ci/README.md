# DRY CI Runner Labels and Rust Toolchain Pins

This directory defines a single-source-of-truth pattern for GitHub Actions runner
images and Rust toolchain versions without introducing moving runner labels
(`ubuntu-latest`), floating toolchain channels (`stable`), or third-party
toolchain actions (`dtolnay/rust-toolchain`).

## Components

1. `config/ci/pins.toml` — declares:
   - `[runners.hosted]`: explicit GitHub-hosted runner images (`linux-x64`,
     `macos-arm64`, `windows-x64`)
   - `[runners].estate_allowlist`: explicit organization-managed runner labels
   - `[rust]`: exact `X.Y.Z` Rust toolchain versions (`msrv`, `stable`), checked
     against `Cargo.toml` `workspace.package.rust-version`
2. `.github/actions/setup-rust/action.yml` — composite `bash` action that
   installs an exact `X.Y.Z` toolchain with `rustup --profile minimal --no-self-update`
   and optional targets/components, with zero third-party action dependencies.
3. `scripts/ci-pins.py` — stdlib `tomllib` + `scripts/workflow_yaml.py` sync and
   verification tool (`sync` and `check` modes).

## Workflow Marker Syntax

In `.github/workflows/*.yml`, annotate hosted runner lines and Rust toolchain
inputs with trailing `# pin:...` comments:

```yaml
jobs:
  lint:
    runs-on: ubuntu-24.04  # pin:runner=linux-x64
    steps:
      - uses: actions/checkout@<40-hex-sha>  # v7
      - uses: ./.github/actions/setup-rust
        with:
          toolchain: 1.98.1  # pin:rust=stable
          components: clippy
```

Matrix axes and `include` rows use the same trailing comment on each hosted
runner value, so `runs-on: ${{ matrix.os }}` remains statically verifiable:

```yaml
    strategy:
      matrix:
        os:
          - ubuntu-24.04  # pin:runner=linux-x64
          - macos-15  # pin:runner=macos-arm64
          - windows-2025  # pin:runner=windows-x64
```

For container jobs that bootstrap `rustup` directly (such as Alpine musl
containers), bind `RUST_TOOLCHAIN` in `env:` with `# pin:rust=msrv` and pass
`--default-toolchain "$RUST_TOOLCHAIN"`.

## Updating a Runner Image or Rust Toolchain

1. Edit `config/ci/pins.toml` (and `Cargo.toml` `rust-version` when bumping
   `msrv`).
2. Run `make ci-pins-sync` — updates every `# pin:...` line across
   `.github/workflows/*.yml`, refreshes `config/release/release-workflow.sha256`
   when present, and runs the full pin check.
3. Review `git diff` and run `make workflow-pin ci-pins-check`.

## Adopting in Another Repository

1. Copy `config/ci/pins.toml`, `.github/actions/setup-rust/action.yml`,
   `scripts/ci-pins.py`, and `scripts/workflow_yaml.py` (if not already present).
   Note: if the target repository uses `rust-toolchain.toml`, ensure its
   `channel` matches `[rust].msrv` or `[rust].stable`; repositories whose
   release-source policy forbids `rust-toolchain.toml` rely solely on
   `config/ci/pins.toml` + `Cargo.toml`.
2. Add `# pin:runner=<key>` and `./.github/actions/setup-rust` (`# pin:rust=<key>`)
   to `.github/workflows/*.yml`, then run `python3 scripts/ci-pins.py sync`.
3. Add `ci-pins-sync` and `ci-pins-check` targets to `Makefile` and wire
   `python3 -B -P -E scripts/ci-pins.py check` into CI and `scripts/workflow-pins.test.sh`.
