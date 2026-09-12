# Contributing

Thanks for your interest. This document explains how the repository is
organised and what a good contribution looks like.

## Ground rules

* **No geometry.** Anything that needs to evaluate a curve or surface is
  out of scope, permanently. See `docs/ARCHITECTURE.md`.
* **No `unsafe`.** The crate forbids it.
* **Read `docs/ARCHITECTURE.md`** before touching `src/model` or `src/p21`.
  Several non-obvious constraints are documented there because they were
  learned the hard way.

## Getting set up

```console
$ git clone https://github.com/jchultarsky/stepq
$ cd stepq
$ tools/fetch-fixtures.sh      # downloads public test files into tests/fixtures/
$ cargo test --all-features
```

The fixtures are not vendored. `tools/fetch-fixtures.sh` pulls the NIST
MBE PMI test set (US-Government work, public domain) and the STEP Tools
AS1 sample assemblies (redistribution terms unstated, so we fetch rather
than copy). Tests that need a fixture skip themselves if it is missing.

## Before you open a pull request

CI runs exactly these; run them locally first:

```console
$ cargo fmt --all -- --check
$ cargo clippy --all-targets --all-features -- -D warnings
$ cargo clippy --no-default-features -- -D warnings
$ cargo test --all-features
$ cargo doc --no-deps --all-features   # with RUSTDOCFLAGS="-D warnings"
$ cargo deny check
```

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
  model/          entity graph, forward + back indices (semantics)
  bin/stepq.rs    the CLI, behind the `cli` feature
tests/            integration tests; fixtures fetched into tests/fixtures/
tools/            developer scripts (fixture fetching, OCCT verification)
docs/             ARCHITECTURE.md and design notes
```

## Reporting bugs

Use the issue template. A bug report about a specific file is only
actionable if you can attach the file, or a minimal file that reproduces
it. If the file is confidential, `stepq strip --anonymize` (once it
exists) is meant for exactly this.
