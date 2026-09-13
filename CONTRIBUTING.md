# Contributing

Thanks for your interest. This document explains how the repository is
organised and what a good contribution looks like.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ground rules

* **No geometry.** Anything that needs to evaluate a curve or surface is
  out of scope, permanently. See `docs/ARCHITECTURE.md`.
* **No `unsafe`.** The crate forbids it.
* **Read `docs/ARCHITECTURE.md`** before touching `src/model` or `src/p21`.
  Several non-obvious constraints are documented there because they were
  learned the hard way.

## Getting set up

You need Rust 1.85 or newer (the MSRV, checked in CI) and, for the
dependency audit, [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny)
(`cargo install cargo-deny`).

```console
$ git clone https://github.com/jchultarsky/stepq
$ cd stepq
$ tools/fetch-fixtures.sh      # downloads public test files into tests/fixtures/
$ tools/fetch-schemas.sh       # downloads EXPRESS schemas into tests/schemas/
$ cargo test --all-features
```

The fixtures are not vendored. `tools/fetch-fixtures.sh` pulls the NIST
MBE PMI test set (US-Government work, public domain), the STEP Tools AS1
sample assemblies and the Open Compute Project's Open Rack and Project
Olympus assemblies (redistribution terms unstated or not granted, so we
fetch rather than copy), and checks each download against a pinned
SHA-256. The Open Compute set is large (~130 MB download, ~860 MB
unpacked, files up to 3.5 million instances); fetch only the small set
with `tools/fetch-fixtures.sh core`. Tests that need a fixture skip
themselves if it is missing, and skip fixtures over 16 MB unless
`STEPQ_LARGE_FIXTURES=1` is set; with it, run the tests in release mode
(`cargo test --release --all-features`), which takes about a minute and
up to 8 GB of memory instead of six minutes in a debug build.

The EXPRESS schemas are not vendored either. They are ISO copyright: ISO
permits using them unmodified for the purposes of the standard, but does
not clearly permit redistribution. Do not commit a schema, or a table
generated from one, without that being settled first.
`tools/fetch-schemas.sh` checks every download against a pinned SHA-256.

## Before you open a pull request

CI runs exactly these; run them locally first:

```console
$ cargo fmt --all -- --check
$ cargo clippy --all-targets --all-features -- -D warnings
$ cargo clippy --no-default-features -- -D warnings
$ cargo test --all-features
$ cargo test --no-default-features --lib
$ RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
$ cargo deny check
```

Changes to closure or extraction logic must also keep the OCCT volume
invariant (`tools/verify-occt.py`, see `docs/ARCHITECTURE.md`) green on the
AS1 and NIST fixtures. The script runs through
[uv](https://docs.astral.sh/uv/), which installs Open CASCADE's Python
bindings (`cadquery-ocp`) on first use:

```console
$ tools/verify-occt.py summary tests/fixtures/steptools/as1-ug-214.stp
$ tools/verify-occt.py compare INPUT OUTPUT...
$ tools/verify-rewrite.sh      # rewrite every fixture and compare, as CI does
$ tools/verify-split.py        # split every fixture and compare, as CI does
```

### Fuzzing

The lexer and parser are fuzzed with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz), which needs a nightly
toolchain. CI runs each target for 60 seconds; to fuzz for longer locally:

```console
$ rustup toolchain install nightly
$ cargo install cargo-fuzz
$ cargo +nightly fuzz run parse -- -dict=fuzz/p21.dict
```

There are two targets. `lex` checks that tokens stay in order and in
bounds and that string decoding never panics. `parse` checks that parsing,
the graph, the query views and `info::Info` never panic, and that writer
output parses back with the same number of instances. A crash found by
fuzzing is a security issue; see [SECURITY.md](SECURITY.md).

Pedantic clippy is on. If a lint is wrong for a specific line, `allow` it
there with a comment; do not disable it crate-wide without discussion.

## Commit and PR conventions

* One logical change per PR. Small PRs get reviewed; large ones get
  postponed.
* Write the commit message for the person running `git blame` in two
  years: say *why*, not just what.
* Add a line to `CHANGELOG.md` under **Unreleased** for anything a user
  would notice.
* New parser behaviour needs a test against a real file, not a synthetic
  one, wherever a real file exists that exercises it.

## Layout

```
src/
  lib.rs          crate root, re-exports
  error.rs        the single Error enum
  p21/            ISO 10303-21 lexer, parser, writer   (syntax only)
  express/        EXPRESS schema reader and attribute-count checks
  model/          entity graph, product structure, extraction (semantics)
  info.rs         the summary behind `stepq info`
  bin/stepq.rs    the CLI, behind the `cli` feature
examples/         `rewrite`, used by tools/verify-rewrite.sh
fuzz/             cargo-fuzz targets
tests/            integration tests; fixtures fetched into tests/fixtures/
tools/            fixture and schema fetching, OCCT verification
docs/             ARCHITECTURE.md and design notes
```

## Releasing

Releases are cut by [dist](https://opensource.axo.dev/cargo-dist/)
(`dist-workspace.toml`, `.github/workflows/release.yml`). Pushing a tag
`vX.Y.Z` builds binaries for macOS, Linux and Windows, shell and
PowerShell installers and an updater, and publishes a GitHub release
whose notes are the matching `CHANGELOG.md` section.

1. On a branch: bump `version` in `Cargo.toml`, move **Unreleased** in
   `CHANGELOG.md` to `## [X.Y.Z] - YYYY-MM-DD`, run `dist plan`, merge.
2. `cargo publish` from the merged `main`.
3. `git tag vX.Y.Z && git push origin vX.Y.Z`.

After changing `dist-workspace.toml` or upgrading dist, run
`dist generate` and commit the regenerated workflow; CI fails otherwise.

## Reporting bugs

Use the issue template. A bug report about a specific file is only
actionable if you can attach the file, or a minimal file that reproduces
it. If the file is confidential, `stepq strip --anonymize` (once it
exists) is meant for exactly this.

Panics, unbounded memory use or hangs on crafted input are security
issues: report them privately as described in [SECURITY.md](SECURITY.md),
not in a public issue.

## Licensing

`stepq` is licensed under the [MIT License](LICENSE). By submitting a pull
request you agree that your contribution is licensed under the same terms,
and you confirm that you have the right to license it that way. Do not add
code copied from projects under incompatible licenses, and do not commit
STEP files — fetch them in `tools/fetch-fixtures.sh` instead.
